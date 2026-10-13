#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 8 (Validierung, Fehlercodes, Rechte): Regressionstests über die GraphQL-Schicht.

use async_graphql::{Request, Variables};
use lineal_api::graphql::{LinealSchema, build_schema};
use lineal_app::RequestCtx;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup(pool: sqlx::PgPool) -> LinealSchema {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    build_schema(test_app(Arc::new(store)))
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

const VIEWER_UPDATE: &str =
    "mutation($i: ViewerUpdateInput!) { viewerUpdate(input: $i) { success viewer { name } } }";
const CREATE_WS: &str =
    "mutation($i: CreateWorkspaceInput!) { createWorkspace(input: $i) { success } }";
const TEAM_CREATE: &str = "mutation($i: TeamCreateInput!) { teamCreate(input: $i) { success } }";
const LABEL_CREATE: &str =
    "mutation($i: IssueLabelCreateInput!) { issueLabelCreate(input: $i) { success } }";

/// B8.01: Namen mit Zeilenumbruch, Bidi-/Zero-Width-Zeichen werden überall abgelehnt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn names_reject_line_breaks_and_invisible_characters(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for bad in [
        "X\nERROR: fake line",
        "a\u{2028}b",
        "a\u{202E}b",
        "a\u{200B}b",
    ] {
        let alice = || ctx_for(SeedUser::Alice);
        let r = run(&schema, alice(), VIEWER_UPDATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "viewerUpdate {bad:?}");
        let r = run(&schema, alice(), CREATE_WS, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "createWorkspace {bad:?}");
        let r = run(&schema, alice(), TEAM_CREATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "teamCreate {bad:?}");
        let r = run(&schema, alice(), LABEL_CREATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "issueLabelCreate {bad:?}");
    }
    // Normalisierung: NFD wird als NFC gespeichert.
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        VIEWER_UPDATE,
        json!({"i": {"name": "Cafe\u{301} Alice"}}),
    )
    .await;
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let v = r.data.into_json().unwrap();
    assert_eq!(v["viewerUpdate"]["viewer"]["name"], "Caf\u{e9} Alice");
}
