#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `teamUpdate` mit Estimates- und Automations-Feldern (WP27).

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

const UPDATE: &str = "mutation($i: TeamUpdateInput!) { teamUpdate(id: \"ENG\", input: $i) { team { issueEstimationType issueEstimationExtended issueEstimationAllowZero defaultIssueEstimate inheritIssueEstimation autoClosePeriod autoCloseStateId autoArchivePeriod autoCloseParentIssues autoCloseChildIssues } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn estimates_and_automations_roundtrip(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"i": {
            "issueEstimationType": "tShirt", "issueEstimationExtended": true,
            "issueEstimationAllowZero": true, "defaultIssueEstimate": 0,
            "autoClosePeriod": 6, "autoCloseStateId": "Canceled",
            "autoArchivePeriod": 3, "autoCloseParentIssues": true, "autoCloseChildIssues": true
        }}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = res.data.into_json().unwrap()["teamUpdate"]["team"].clone();
    assert_eq!(t["issueEstimationType"], "tShirt");
    assert_eq!(t["issueEstimationAllowZero"], true);
    assert_eq!(t["autoClosePeriod"], 6);
    assert_eq!(t["autoArchivePeriod"], 3);
    assert_eq!(t["autoCloseParentIssues"], true);
    assert!(t["autoCloseStateId"].is_string());

    // 0 schaltet Auto-close aus; ungültige Werte werden abgelehnt.
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"i": {"autoClosePeriod": 0}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = res.data.into_json().unwrap()["teamUpdate"]["team"].clone();
    assert!(t["autoClosePeriod"].is_null());

    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"i": {"autoArchivePeriod": 5}}),
    )
    .await;
    assert!(!res.errors.is_empty());
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"i": {"issueEstimationType": "bogus"}}),
    )
    .await;
    assert!(!res.errors.is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn default_templates_are_readable(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let res = run(
        &schema,
        alice(),
        "mutation { templateCreate(input: { type: issue, name: \"Bug\", teamId: \"ENG\", templateData: {title: \"Bug: \"} }) { template { id } } }",
        json!({}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.data.into_json().unwrap()["templateCreate"]["template"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let res = run(
        &schema,
        alice(),
        "mutation($t: String) { teamDefaultTemplateSet(teamId: \"ENG\", slot: forMembers, templateId: $t) }",
        json!({"t": id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = run(
        &schema,
        alice(),
        "{ teamDefaultTemplates(teamId: \"ENG\") { forMembersId forNonMembersId projectId } }",
        json!({}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let d = res.data.into_json().unwrap()["teamDefaultTemplates"].clone();
    assert_eq!(d["forMembersId"], json!(id));
    assert!(d["forNonMembersId"].is_null());
}
