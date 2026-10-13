#![allow(clippy::unwrap_used, clippy::expect_used)]
//! F.01: Projektmitglieder per `projectUpdate(addMembers/removeMembers)` und beim Anlegen.

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

fn member_names(d: &Value) -> Vec<String> {
    let mut v: Vec<String> = d["project"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_lowercase())
        .collect();
    v.sort();
    v
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn members_can_be_added_and_removed(pool: PgPool) {
    let schema = setup(pool).await;
    let bob = SeedData::acme().bob.to_string();
    let carol = SeedData::acme().carol.to_string();
    let q = "{ project(id: \"P-ENG-1\") { lead { id } members { id name } } }";
    let (d, _) = run(&schema, SeedUser::Alice, q).await;
    let before = member_names(&d);
    assert!(!before.contains(&"bob".to_owned()), "{before:?}");

    let add = |u: &str| {
        format!(
            "mutation {{ projectUpdate(id: \"P-ENG-1\", input: {{addMembers: [\"{u}\"]}}) {{ success }} }}"
        )
    };
    let rm = |u: &str| {
        format!(
            "mutation {{ projectUpdate(id: \"P-ENG-1\", input: {{removeMembers: [\"{u}\"]}}) {{ success }} }}"
        )
    };
    let (_, e) = run(&schema, SeedUser::Alice, &add(&bob)).await;
    assert!(e.is_empty(), "{e:?}");
    let (d, _) = run(&schema, SeedUser::Alice, q).await;
    assert!(
        member_names(&d).iter().any(|n| n.contains("bob")),
        "{:?} {d}",
        member_names(&d)
    );

    // Ein Gast ohne Zugriff auf ein Projekt-Team kann nicht Mitglied werden.
    let (_, e) = run(&schema, SeedUser::Alice, &add(&carol)).await;
    assert!(!e.is_empty());

    let (_, e) = run(&schema, SeedUser::Alice, &rm(&bob)).await;
    assert!(e.is_empty(), "{e:?}");
    let (d, _) = run(&schema, SeedUser::Alice, q).await;
    assert_eq!(member_names(&d), before);

    // Der Lead bleibt Mitglied.
    if let Some(lead) = d["project"]["lead"]["id"].as_str() {
        let (_, e) = run(&schema, SeedUser::Alice, &rm(lead)).await;
        assert!(!e.is_empty());
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn members_on_create(pool: PgPool) {
    let schema = setup(pool).await;
    let bob = SeedData::acme().bob.to_string();
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        &format!(
            "mutation {{ projectCreate(input: {{name: \"Mit Team\", addTeams: [\"ENG\"], addMembers: [\"{bob}\"]}}) {{ project {{ members {{ id }} }} }} }}"
        ),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(
        d["projectCreate"]["project"]["members"][0]["id"],
        bob.as_str()
    );
}
