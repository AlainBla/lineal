#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP29: persönlicher Account über den echten Router – `viewer`, `viewerUpdate`, `passwordChange`,
//! `sessions`, `sessionRevoke`; Cookie-Sessions, CSRF-Header, Fehlversuchs-Zähler.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use lineal_app::App;
use lineal_app::ports::Store;
use lineal_app::usecases::auth::CreateApiKeyInput;
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

/// Meldet per `/auth/login` an und liefert den Cookie-Wert `lineal_session=<token>`.
async fn login(env: &Env, email: &str, password: &str) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header("content-type", "application/json")
        .header("origin", "http://localhost:8080")
        .body(Body::from(
            json!({ "email": email, "password": password }).to_string(),
        ))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let set = res.headers().get(header::SET_COOKIE).unwrap();
    set.to_str().unwrap().split(';').next().unwrap().to_owned()
}

enum Auth<'a> {
    Cookie(&'a str),
    Header(&'a str),
}

async fn gql(env: &Env, auth: Auth<'_>, query: &str, vars: Value) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/graphql")
        .header("content-type", "application/json")
        .header("origin", "http://localhost:8080")
        .header("x-lineal-csrf", "1");
    b = match auth {
        Auth::Cookie(c) => b.header(header::COOKIE, c),
        Auth::Header(k) => b.header(header::AUTHORIZATION, k),
    };
    let req = b
        .body(Body::from(
            json!({ "query": query, "variables": vars }).to_string(),
        ))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn code(v: &Value) -> &str {
    v["errors"][0]["extensions"]["code"].as_str().unwrap_or("")
}

const SESSIONS: &str = "{ sessions { id current userAgent createdAt expiresAt } }";
const CHANGE: &str = "mutation($o: String!, $n: String!) { passwordChange(currentPassword: $o, newPassword: $n) { success revokedSessions } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn password_change_signs_out_the_other_session_over_http(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let a = login(&env, "alice@acme.example", "correct-horse").await;
    let b = login(&env, "alice@acme.example", "correct-horse").await;

    // Beide Sessions sehen zwei Einträge; „current“ zeigt je auf die eigene.
    let (_, va) = gql(&env, Auth::Cookie(&a), SESSIONS, json!({})).await;
    let (_, vb) = gql(&env, Auth::Cookie(&b), SESSIONS, json!({})).await;
    let list_a = va["data"]["sessions"].as_array().unwrap();
    let list_b = vb["data"]["sessions"].as_array().unwrap();
    assert_eq!(list_a.len(), 2);
    let cur = |l: &Vec<Value>| l.iter().find(|s| s["current"] == true).unwrap()["id"].clone();
    assert_ne!(cur(list_a), cur(list_b));
    assert_eq!(list_a.iter().filter(|s| s["current"] == true).count(), 1);

    // Falsches altes Passwort: Fehler; die andere Session lebt.
    let (_, v) = gql(
        &env,
        Auth::Cookie(&a),
        CHANGE,
        json!({ "o": "nope", "n": "brand-new-pw" }),
    )
    .await;
    assert_eq!(code(&v), "invalid_request");
    assert_eq!(
        gql(&env, Auth::Cookie(&b), SESSIONS, json!({})).await.0,
        StatusCode::OK
    );

    // Richtig: die andere Session ist danach ungültig, die eigene gültig.
    let (_, v) = gql(
        &env,
        Auth::Cookie(&a),
        CHANGE,
        json!({ "o": "correct-horse", "n": "brand-new-pw" }),
    )
    .await;
    assert_eq!(v["data"]["passwordChange"]["success"], true, "{v}");
    assert_eq!(v["data"]["passwordChange"]["revokedSessions"], 1);
    assert_eq!(
        gql(&env, Auth::Cookie(&b), SESSIONS, json!({})).await.0,
        StatusCode::UNAUTHORIZED,
        "old session is invalid"
    );
    assert_eq!(
        gql(&env, Auth::Cookie(&a), SESSIONS, json!({})).await.0,
        StatusCode::OK
    );
    // Neues Passwort loggt ein, altes nicht.
    let _ = login(&env, "alice@acme.example", "brand-new-pw").await;
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_keys_cannot_change_passwords_or_manage_sessions(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let key = env
        .app
        .create_api_key(
            &ctx_for(SeedUser::Alice),
            CreateApiKeyInput {
                label: "ci".into(),
                scopes: vec!["read".into(), "write".into(), "admin".into()],
                teams: None,
                expires_at: None,
            },
        )
        .await
        .unwrap()
        .key;
    let (_, v) = gql(
        &env,
        Auth::Header(&key),
        CHANGE,
        json!({ "o": "correct-horse", "n": "brand-new-pw" }),
    )
    .await;
    assert_eq!(code(&v), "forbidden", "{v}");
    let (_, v) = gql(&env, Auth::Header(&key), SESSIONS, json!({})).await;
    assert_eq!(code(&v), "forbidden", "{v}");
    let (_, v) = gql(
        &env,
        Auth::Header(&key),
        "mutation { sessionsRevokeOthers { count } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&v), "forbidden", "{v}");
    // Das Passwort ist unverändert.
    let _ = login(&env, "alice@acme.example", "correct-horse").await;
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn foreign_sessions_are_not_found_and_stay_alive(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let a = login(&env, "alice@acme.example", "correct-horse").await;
    let b = login(&env, "bob@acme.example", "correct-horse").await;
    let (_, vb) = gql(&env, Auth::Cookie(&b), SESSIONS, json!({})).await;
    let bob_session = vb["data"]["sessions"][0]["id"].as_str().unwrap().to_owned();
    let (_, v) = gql(
        &env,
        Auth::Cookie(&a),
        "mutation($id: String!) { sessionRevoke(id: $id) { success } }",
        json!({ "id": bob_session }),
    )
    .await;
    assert_eq!(code(&v), "not_found", "{v}");
    assert_eq!(
        gql(&env, Auth::Cookie(&b), SESSIONS, json!({})).await.0,
        StatusCode::OK
    );
    // Unbekannte und ungültige IDs verhalten sich gleich.
    for id in ["not-a-uuid", "00000000-0000-7000-8000-000000000000"] {
        let (_, v) = gql(
            &env,
            Auth::Cookie(&a),
            "mutation($id: String!) { sessionRevoke(id: $id) { success } }",
            json!({ "id": id }),
        )
        .await;
        assert_eq!(code(&v), "not_found", "{id}");
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn repeated_wrong_passwords_are_throttled(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let a = login(&env, "alice@acme.example", "correct-horse").await;
    for i in 0..10 {
        let (_, v) = gql(
            &env,
            Auth::Cookie(&a),
            CHANGE,
            json!({ "o": format!("wrong-{i}"), "n": "brand-new-pw" }),
        )
        .await;
        assert_eq!(code(&v), "invalid_request", "attempt {i}: {v}");
    }
    // Ab jetzt gesperrt – auch mit dem richtigen Passwort.
    let (_, v) = gql(
        &env,
        Auth::Cookie(&a),
        CHANGE,
        json!({ "o": "correct-horse", "n": "brand-new-pw" }),
    )
    .await;
    assert_eq!(code(&v), "rate_limited", "{v}");
    assert!(
        v["errors"][0]["extensions"]["details"]
            .as_str()
            .unwrap()
            .contains("retryAfter")
    );
    // Das Passwort blieb unverändert; Bob ist nicht betroffen.
    let _ = login(&env, "alice@acme.example", "correct-horse").await;
    let b = login(&env, "bob@acme.example", "correct-horse").await;
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        CHANGE,
        json!({ "o": "correct-horse", "n": "bobs-new-pw1" }),
    )
    .await;
    assert_eq!(v["data"]["passwordChange"]["success"], true, "{v}");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn viewer_and_profile_update_work_for_the_caller_only(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let b = login(&env, "bob@acme.example", "correct-horse").await;
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        "{ viewer { id name displayName email role timezone avatarUrl } }",
        json!({}),
    )
    .await;
    assert_eq!(v["data"]["viewer"]["email"], "bob@acme.example");
    assert_eq!(v["data"]["viewer"]["id"], SeedData::acme().bob.to_string());
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        "mutation($i: ViewerUpdateInput!) { viewerUpdate(input: $i) { viewer { name displayName timezone } } }",
        json!({ "i": { "name": "Robert", "displayName": "rob", "timezone": "Europe/Berlin" } }),
    )
    .await;
    assert_eq!(v["data"]["viewerUpdate"]["viewer"]["name"], "Robert", "{v}");
    // Nichts außer dem eigenen Profil ist über die Eingabe erreichbar (keine User-ID im Schema).
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        "mutation { viewerUpdate(input: { userId: \"x\", name: \"Evil\" }) { success } }",
        json!({}),
    )
    .await;
    assert!(v["errors"].is_array(), "{v}");
    // Zeitzone mit null leeren; vergebener Anzeigename → conflict.
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        "mutation { viewerUpdate(input: { timezone: null }) { viewer { timezone } } }",
        json!({}),
    )
    .await;
    assert!(
        v["data"]["viewerUpdate"]["viewer"]["timezone"].is_null(),
        "{v}"
    );
    let (_, v) = gql(
        &env,
        Auth::Cookie(&b),
        "mutation { viewerUpdate(input: { displayName: \"alice\" }) { success } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&v), "conflict");
}
