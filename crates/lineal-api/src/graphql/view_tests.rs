//! GraphQL-Contract-Tests für `view`, `search` und `overview` gegen echte Datenbank.

use crate::graphql::build_schema;
use lineal_app::usecases::overview::OverviewInput;
use lineal_app::ports::Store;
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (Arc<App>, sqlx::PgPool) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (test_app(Arc::new(store)), pool)
}

async fn issue(pool: &sqlx::PgPool, team: lineal_domain::ids::TeamId, slot: u128, number: i32, title: &str) {
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(Uuid::now_v7())
    .bind(SeedData::acme().org.0)
    .bind(team.0)
    .bind(number)
    .bind(title)
    .bind(seed_workflow_state_ids(slot)[1].0)
    .execute(pool)
    .await
    .unwrap();
}

async fn run(app: &Arc<App>, ctx: RequestCtx, query: &str, vars: Value) -> async_graphql::Response {
    build_schema(app.clone())
        .execute(
            async_graphql::Request::new(query)
                .variables(async_graphql::Variables::from_json(vars))
                .data(ctx),
        )
        .await
}

fn data(res: async_graphql::Response) -> Value {
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.data.into_json().unwrap()
}

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected an error");
    serde_json::to_value(e.extensions.as_ref().expect("extensions")).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn custom_views_over_graphql(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let d = SeedData::acme();
    issue(&pool, d.eng, 1, 101, "Login fails").await;
    let bob = ctx_for(SeedUser::Bob);
    let created = data(
        run(
            &app,
            bob.clone(),
            "mutation($i: CustomViewCreateInput!) { customViewCreate(input: $i) { success customView { id slugId name modelName canEdit filterData } } }",
            json!({"i": {"name": "Prio", "modelName": "Issue", "filterData": {"priority": {"eq": 0}}}}),
        )
        .await,
    );
    let view = &created["customViewCreate"]["customView"];
    assert_eq!(view["filterData"]["priority"]["in"], json!([0]), "normalized");
    let id = view["id"].as_str().unwrap().to_owned();
    // Anwenden über viewIssues.
    let r = data(
        run(
            &app,
            bob.clone(),
            "query($v: String) { viewIssues(customViewId: $v) { nodes { identifier } hasNextPage } }",
            json!({"v": id}),
        )
        .await,
    );
    assert_eq!(r["viewIssues"]["nodes"][0]["identifier"], "ENG-101");
    // Fremde persönliche View → not_found.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        "query($v: String!) { customView(id: $v) { id } }",
        json!({"v": id}),
    )
    .await;
    assert_eq!(code(&res), "not_found");
    // Fehlerhafter Filter → invalid_request.
    let res = run(
        &app,
        bob,
        "mutation($i: CustomViewCreateInput!) { customViewCreate(input: $i) { success } }",
        json!({"i": {"name": "Bad", "modelName": "Issue", "filterData": {"nope": 1}}}),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn search_and_overview_over_graphql(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let d = SeedData::acme();
    issue(&pool, d.eng, 1, 101, "Zebra public").await;
    issue(&pool, d.des, 3, 101, "Zebra secret").await;
    let q = "query { searchIssues(term: \"zebra\") { nodes { issue { identifier } highlights { field snippet } } hasNextPage } }";
    let a = data(run(&app, ctx_for(SeedUser::Alice), q, json!({})).await);
    let ids: Vec<&str> = a["searchIssues"]["nodes"].as_array().unwrap().iter().map(|n| n["issue"]["identifier"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ENG-101"]);
    assert!(a["searchIssues"]["nodes"][0]["highlights"][0]["snippet"].as_str().unwrap().contains("<mark>"));
    let b = data(run(&app, ctx_for(SeedUser::Bob), q, json!({})).await);
    assert_eq!(b["searchIssues"]["nodes"].as_array().unwrap().len(), 2);
    // Übersicht: gleiche Zahlen wie der Use-Case (und damit wie MCP).
    let o = data(
        run(
            &app,
            ctx_for(SeedUser::Alice),
            "query { workspaceOverview { totals { openIssues } teams { key counts { openIssues } } } workload { user { name } } activeCycles { teamKey } }",
            json!({}),
        )
        .await,
    );
    let direct = app
        .workspace_overview(&ctx_for(SeedUser::Alice), OverviewInput::default())
        .await
        .unwrap();
    assert_eq!(o["workspaceOverview"]["totals"]["openIssues"], direct.totals.open_issues);
    assert_eq!(o["workspaceOverview"]["teams"][0]["counts"]["openIssues"], direct.teams[0].counts.open_issues);
    // Gast → forbidden.
    let res = run(&app, ctx_for(SeedUser::Carol), "query { workspaceOverview { totals { openIssues } } }", json!({})).await;
    assert_eq!(code(&res), "forbidden");
}
