#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `Team.memberCount`/`allMembersCanJoin`, `teamCreate`/`teamUpdate` (WP26).

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

const CREATE: &str = "mutation($i: TeamCreateInput!) { teamCreate(input: $i) { team { id key allMembersCanJoin memberCount } } }";
const UPDATE: &str = "mutation($id: String!, $i: TeamUpdateInput!) { teamUpdate(id: $id, input: $i) { team { allMembersCanJoin } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn create_and_update_expose_join_flag_and_member_count(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        CREATE,
        json!({"i": {"name": "KollegePlus", "key": "KP", "allMembersCanJoin": false}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let team = &v["teamCreate"]["team"];
    assert_eq!(team["key"], "KP");
    assert_eq!(team["allMembersCanJoin"], false);
    assert_eq!(team["memberCount"], 1);
    let id = team["id"].as_str().unwrap().to_owned();

    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"id": id, "i": {"allMembersCanJoin": true}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["teamUpdate"]["team"]["allMembersCanJoin"], true);
}
