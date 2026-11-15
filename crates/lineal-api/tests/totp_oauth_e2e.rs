#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-70 T14: Der OAuth-Authorize-Flow (MCP/DCR) ist durch das Login-Gate
//! automatisch abgedeckt. Anonym leitet `/oauth/authorize` zu `/login?next=...`,
//! und nach Passwort **und** TOTP fuehrt dieselbe Anfrage mit Session-Cookie
//! nicht mehr zum Login zurueck.

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
    app.set_user_password(&ctx_for(SeedUser::Bob), "me", "correct-horse")
        .await
        .unwrap();
    let router = lineal_api::router(app.clone(), false);
    Env { app, router }
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_client(env: &Env) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "client_name": "Test", "redirect_uris": ["http://127.0.0.1:1234/cb"] })
                .to_string(),
        ))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    body_json(res).await["client_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn authorize_uri(client_id: &str) -> String {
    // code_challenge: 43 Zeichen base64url; resource wird weggelassen (resource_ok erlaubt leer).
    let challenge = "0123456789012345678901234567890123456789012";
    format!(
        "/oauth/authorize?client_id={client_id}&redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2Fcb\
         &response_type=code&code_challenge={challenge}&code_challenge_method=S256&scope=read"
    )
}

async fn get(env: &Env, uri: &str, cookie: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder().method(Method::GET).uri(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    env.router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
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

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn oauth_authorize_is_gated_by_login_and_totp(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let client_id = register_client(&env).await;

    // bob hat TOTP aktiv.
    let enrollment = env
        .app
        .totp_begin_enrollment(&ctx_for(SeedUser::Bob))
        .await
        .unwrap();
    let secret = core::base32_decode(&enrollment.secret_base32).unwrap();
    env.app
        .totp_confirm_enrollment(
            &ctx_for(SeedUser::Bob),
            &core::code_now(&secret, env.app.clock.now().timestamp()),
            EnrollSession::RevokeOthers {
                keep_token_hash: None,
            },
        )
        .await
        .unwrap();

    let uri = authorize_uri(&client_id);

    // Anonym: Weiterleitung zum Login mit next-Parameter.
    let res = get(&env, &uri, None).await;
    assert!(res.status().is_redirection());
    let loc = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(loc.contains("/login"), "anonym muss zum Login, war {loc}");
    assert!(loc.contains("next="), "next-Parameter muss gesetzt sein");

    // Passwort -> totpRequired -> zweiter Schritt -> Session-Cookie.
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
        json!({ "pendingToken": token, "code": core::code_now(&secret, env.app.clock.now().timestamp()) }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
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

    // Mit der (nach TOTP geprägten) Session fuehrt dieselbe Anfrage NICHT mehr zum Login.
    let res = get(&env, &uri, Some(&cookie)).await;
    if let Some(loc) = res.headers().get(header::LOCATION) {
        assert!(
            !loc.to_str().unwrap().contains("/login"),
            "mit Session darf nicht zum Login zurueckgeleitet werden"
        );
    } else {
        // Kein Redirect = Consent-Seite gerendert (auch gut).
        assert_eq!(res.status(), StatusCode::OK);
    }
}
