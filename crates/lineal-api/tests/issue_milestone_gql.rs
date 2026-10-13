#![allow(clippy::unwrap_used, clippy::expect_used)]
//! F.07: Meilenstein am Issue per `issueBatchUpdate` (Bulk-Leiste) und `issueUpdate`.

use async_graphql::Request;
use lineal_api::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;

async fn run(
    schema: &lineal_api::graphql::LinealSchema,
    who: SeedUser,
    q: &str,
) -> (Value, Vec<String>) {
    let res = schema.execute(Request::new(q).data(ctx_for(who))).await;
    let errs = res.errors.iter().map(|e| e.message.clone()).collect();
    (res.data.into_json().unwrap(), errs)
}

async fn setup(pool: PgPool) -> lineal_api::graphql::LinealSchema {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    build_schema(test_app(Arc::new(store)))
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn batch_sets_and_clears_the_milestone(pool: PgPool) {
    let schema = setup(pool).await;
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ project(id: \"P-ENG-1\") { id milestones { id name } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let pid = d["project"]["id"].as_str().unwrap().to_owned();
    let mid = d["project"]["milestones"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        &format!(
            "mutation {{ issueCreate(input: {{teamId: \"ENG\", title: \"Mit Meilenstein\", projectId: \"{pid}\"}}) {{ issue {{ id }} }} }}"
        ),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let id = d["issueCreate"]["issue"]["id"].as_str().unwrap().to_owned();

    let set = |m: &str| {
        format!(
            "mutation {{ issueBatchUpdate(input: {{ids: [\"{id}\"], projectMilestoneId: {m}}}) {{ issues {{ id projectMilestoneId }} failures {{ code }} }} }}"
        )
    };
    let (d, e) = run(&schema, SeedUser::Alice, &set(&format!("\"{mid}\""))).await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(
        d["issueBatchUpdate"]["issues"][0]["projectMilestoneId"],
        mid.as_str()
    );
    // Erneut lesen (Persistenz) und wieder lösen.
    let (d, _) = run(
        &schema,
        SeedUser::Alice,
        &format!("{{ issue(id: \"{id}\") {{ projectMilestoneId }} }}"),
    )
    .await;
    assert_eq!(d["issue"]["projectMilestoneId"], mid.as_str());
    let (d, e) = run(&schema, SeedUser::Alice, &set("null")).await;
    assert!(e.is_empty(), "{e:?}");
    assert!(d["issueBatchUpdate"]["issues"][0]["projectMilestoneId"].is_null());
    // Ein fremder/unbekannter Meilenstein wird je Issue als Fehler gemeldet, nichts wird gesetzt.
    let (d, _) = run(
        &schema,
        SeedUser::Alice,
        &set("\"00000000-0000-4000-8000-000000000001\""),
    )
    .await;
    assert_eq!(
        d["issueBatchUpdate"]["failures"].as_array().unwrap().len(),
        1
    );
}
