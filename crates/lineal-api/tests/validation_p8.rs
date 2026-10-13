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

const ISSUE_CREATE: &str = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";

/// B8.02: NUL-Zeichen in beliebigen Textfeldern liefern `invalid_request` statt `internal`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn nul_characters_are_invalid_request_not_internal(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        ISSUE_CREATE,
        json!({"i": {"teamId": "ENG", "title": "ok", "description": "a\u{0}b"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        TEAM_CREATE,
        json!({"i": {"name": "Nul Team", "description": "x\u{0}y"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        LABEL_CREATE,
        json!({"i": {"name": "n", "description": "x\u{0}y"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
}

fn ext(res: &async_graphql::Response) -> Value {
    let e = res.errors.first().expect("expected error");
    serde_json::to_value(e).unwrap()["extensions"].clone()
}

/// B8.15: ungültige Eingaben (Argument-Coercion, Enum-Werte, Zeitpunkte) sind `invalid_request`.
/// B8.18: `extensions.details` ist ein Objekt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn invalid_inputs_are_invalid_request_with_object_details(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        "mutation { teamUpdate(id: \"ENG\", input: { issueEstimationType: \"bogus\" }) { success } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    // Details bleiben ein Objekt (kein JSON-String).
    let r = run(
        &schema,
        alice(),
        "mutation($u: String!, $r: String!) { userRoleUpdate(userId: $u, role: $r) { success } }",
        json!({"u": SeedData::acme().bob.to_string(), "r": "superhero"}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    assert!(ext(&r)["details"].is_object(), "{}", ext(&r));
    assert!(ext(&r)["requestId"].is_string());
    let r = run(
        &schema,
        alice(),
        "mutation($i: NotificationUpdateInput!) { notificationUpdate(id: \"00000000-0000-0000-0000-000000000000\", input: $i) { success } }",
        json!({"i": {"snoozedUntilAt": "morgen"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
}

/// B8.16: `first` außerhalb 1..=250 ist überall `invalid_request`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn first_out_of_range_is_invalid_request(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for q in [
        "{ issues(first: 0) { nodes { id } } }",
        "{ issues(first: -1) { nodes { id } } }",
        "{ issues(first: 251) { nodes { id } } }",
        "{ teams(first: 0) { id } }",
        "{ notifications(first: -1) { nodes { id } } }",
    ] {
        let r = run(&schema, ctx_for(SeedUser::Alice), q, json!({})).await;
        assert_eq!(code(&r), "invalid_request", "{q}: {:?}", r.errors);
    }
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        "{ issues(first: 5, after: \"\") { nodes { id } } }",
        json!({}),
    )
    .await;
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// B8.14: Datumseingaben strikt `YYYY-MM-DD` mit plausiblem Jahr.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn dates_are_strict_and_plausible(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for bad in [
        "2026-2-3",
        "+2026-02-03",
        "26-02-03",
        "0001-01-01",
        "9999-12-31",
    ] {
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            "mutation($i: ProjectInput!) { projectCreate(input: $i) { success } }",
            json!({"i": {"name": "P", "setTeams": ["ENG"], "startDate": bad}}),
        )
        .await;
        assert_eq!(code(&r), "invalid_request", "project {bad}: {:?}", r.errors);
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            ISSUE_CREATE,
            json!({"i": {"teamId": "ENG", "title": "t", "dueDate": bad}}),
        )
        .await;
        assert_eq!(code(&r), "invalid_request", "issue {bad}: {:?}", r.errors);
    }
}
