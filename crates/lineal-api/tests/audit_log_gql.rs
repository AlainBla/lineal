#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `auditLog` (WP25): Rechte (nur Owner/Admin mit Scope admin), Cursor-Pagination,
//! Obergrenze 100 je Seite, Filter und keine Geheimnisse in der Antwort.

use async_graphql::{Request, Variables};
use lineal_api::graphql::{LinealSchema, build_schema};
use lineal_app::ports::Store;
use lineal_app::usecases::auth::LoginInput;
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

const PASSWORD: &str = "correct-horse";
const WRONG_PASSWORD: &str = "guessed-password-1";

async fn setup(pool: sqlx::PgPool) -> (LinealSchema, Arc<App>) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    app.set_user_password(&ctx_for(SeedUser::Alice), "me", PASSWORD)
        .await
        .unwrap();
    let schema = build_schema(app.clone());
    (schema, app)
}

async fn run(
    schema: &LinealSchema,
    ctx: RequestCtx,
    query: &str,
    vars: Value,
) -> async_graphql::Response {
    schema
        .execute(
            Request::new(query)
                .variables(Variables::from_json(vars))
                .data(ctx),
        )
        .await
}

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected error");
    let v = serde_json::to_value(e).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

const LOG: &str = "query($first: Int, $after: String, $action: String) { auditLog(first: $first, after: $after, action: $action) { nodes { id action actorName actorKind credentialKind details ip requestId } pageInfo { hasNextPage endCursor } } }";

/// Erzeugt je einen Fehlversuch und einen Erfolg (mit IP) über den Application-Layer.
async fn sign_in_twice(app: &App) -> String {
    let _ = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: WRONG_PASSWORD.into(),
            ..Default::default()
        })
        .await
        .expect_err("wrong password");
    let ok = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: PASSWORD.into(),
            ip_address: Some("203.0.113.9".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    ok.token
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn audit_log_is_for_admins_only_and_paginates(pool: sqlx::PgPool) {
    let (schema, app) = setup(pool).await;
    sign_in_twice(&app).await;

    // Gast und Mitglied: forbidden.
    let res = run(&schema, ctx_for(SeedUser::Carol), LOG, json!({})).await;
    assert_eq!(code(&res), "forbidden");
    let res = run(&schema, ctx_for(SeedUser::Bob), LOG, json!({})).await;
    assert_eq!(code(&res), "forbidden");

    // Owner ohne Admin-Scope: forbidden.
    let mut no_admin = ctx_for(SeedUser::Alice);
    no_admin.scopes.admin = false;
    let res = run(&schema, no_admin, LOG, json!({})).await;
    assert_eq!(code(&res), "forbidden");

    // Owner mit Admin-Scope: erste Seite mit einem Eintrag, dann die nächste.
    let res = run(&schema, ctx_for(SeedUser::Alice), LOG, json!({"first": 1})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["auditLog"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(v["auditLog"]["pageInfo"]["hasNextPage"], true);
    let cursor = v["auditLog"]["pageInfo"]["endCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        LOG,
        json!({"first": 1, "after": cursor}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["auditLog"]["nodes"].as_array().unwrap().len(), 1);

    // Filter nach Aktion.
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        LOG,
        json!({"action": "auth.login_failed"}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let nodes = v["auditLog"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["action"], "auth.login_failed");
    assert_eq!(nodes[0]["actorName"], "Alice Example");
    assert_eq!(nodes[0]["details"], "{\"reason\":\"invalid_password\"}");

    // Seitengröße: 0 und 101 sind ungültig; kaputter Cursor ebenfalls.
    for first in [0, 101] {
        let res = run(
            &schema,
            ctx_for(SeedUser::Alice),
            LOG,
            json!({"first": first}),
        )
        .await;
        assert_eq!(code(&res), "invalid_request", "first={first}");
    }
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        LOG,
        json!({"after": "nicht-gueltig"}),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn audit_response_has_no_secrets_or_emails(pool: sqlx::PgPool) {
    let (schema, app) = setup(pool).await;
    let token = sign_in_twice(&app).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        LOG,
        json!({"first": 100}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let text = serde_json::to_string(&res.data.into_json().unwrap()).unwrap();
    assert!(text.contains("auth.login_succeeded"));
    assert!(text.contains("203.0.113.9"));
    assert!(!text.contains(&token), "session token leaked");
    assert!(!text.contains(PASSWORD));
    assert!(!text.contains(WRONG_PASSWORD));
    assert!(!text.contains("alice@acme.example"), "e-mail leaked");
}
