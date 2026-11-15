#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-70 T9: TOTP über den echten Router — Login-Zweitschritt und Enrollment
//! per Session. Codes werden gegen die (feste) App-Uhr berechnet.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use lineal_app::App;
use lineal_app::ports::Store;
use lineal_app::usecases::totp::core;
use lineal_app::usecases::totp::service::EnrollSession;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Env {
    app: Arc<App>,
    router: Router,
}

async fn setup(pool: sqlx::PgPool) -> Env {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    for user in [SeedUser::Alice, SeedUser::Bob] {
        app.set_user_password(&ctx_for(user), "me", "correct-horse")
            .await
            .unwrap();
    }
    let router = lineal_api::router(app.clone(), false);
    Env { app, router }
}

fn code(env: &Env, secret: &[u8]) -> String {
    core::code_now(secret, env.app.clock.now().timestamp())
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn post(env: &Env, uri: &str, body: Value) -> axum::response::Response {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("origin", "http://localhost:8080")
        .body(Body::from(body.to_string()))
        .unwrap();
    env.router.clone().oneshot(req).await.unwrap()
}

/// Enrollt bob direkt über den Usecase und gibt das Rohsecret zurück.
async fn enroll_bob(env: &Env) -> Vec<u8> {
    let ctx = ctx_for(SeedUser::Bob);
    let enrollment = env.app.totp_begin_enrollment(&ctx).await.unwrap();
    let secret = core::base32_decode(&enrollment.secret_base32).unwrap();
    env.app
        .totp_confirm_enrollment(
            &ctx,
            &code(env, &secret),
            EnrollSession::RevokeOthers {
                keep_token_hash: None,
            },
        )
        .await
        .unwrap();
    secret
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn login_requires_totp_second_step(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let secret = enroll_bob(&env).await;

    // Schritt 1: Passwort-Login liefert totpRequired + Pending-Token, KEIN Cookie.
    let res = post(
        &env,
        "/auth/login",
        json!({ "email": "bob@acme.example", "password": "correct-horse" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get(header::SET_COOKIE).is_none());
    let j = body_json(res).await;
    assert_eq!(j["totpRequired"], json!(true));
    let token = j["pendingToken"].as_str().unwrap().to_string();

    // Schritt 2: Pending-Token + Code ⇒ Session-Cookie.
    let res = post(
        &env,
        "/auth/login/totp",
        json!({ "pendingToken": token, "code": code(&env, &secret) }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get(header::SET_COOKIE).is_some());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn wrong_second_factor_is_rejected(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let _secret = enroll_bob(&env).await;
    let res = post(
        &env,
        "/auth/login",
        json!({ "email": "bob@acme.example", "password": "correct-horse" }),
    )
    .await;
    let token = body_json(res).await["pendingToken"]
        .as_str()
        .unwrap()
        .to_string();
    let res = post(
        &env,
        "/auth/login/totp",
        json!({ "pendingToken": token, "code": "000000" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn enroll_via_session_endpoints(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    // Login ohne TOTP ⇒ Cookie.
    let res = post(
        &env,
        "/auth/login",
        json!({ "email": "bob@acme.example", "password": "correct-horse" }),
    )
    .await;
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // begin (mit Cookie + CSRF-Header).
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/totp/enroll/begin")
        .header("content-type", "application/json")
        .header("origin", "http://localhost:8080")
        .header("x-lineal-csrf", "1")
        .header(header::COOKIE, &cookie)
        .body(Body::from("{}"))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let secret =
        core::base32_decode(body_json(res).await["secretBase32"].as_str().unwrap()).unwrap();

    // confirm.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/totp/enroll/confirm")
        .header("content-type", "application/json")
        .header("origin", "http://localhost:8080")
        .header("x-lineal-csrf", "1")
        .header(header::COOKIE, &cookie)
        .body(Body::from(
            json!({ "code": code(&env, &secret) }).to_string(),
        ))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let codes = body_json(res).await;
    assert_eq!(codes["recoveryCodes"].as_array().unwrap().len(), 10);

    // status zeigt bestätigt.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/auth/totp/status")
        .header("origin", "http://localhost:8080")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["confirmed"], json!(true));
}
