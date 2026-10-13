#![allow(clippy::unwrap_used, clippy::expect_used)]
//! F.04: „Recently deleted“-Listen (Issues, Projekte, Initiativen) mit Sichtbarkeit und Restore.

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
async fn trashed_issues_are_listed_and_restorable(pool: PgPool) {
    let schema = setup(pool).await;
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"Weg damit\"}) { issue { id identifier } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let id = d["issueCreate"]["issue"]["id"].as_str().unwrap().to_owned();
    let (d, _) = run(&schema, SeedUser::Alice, "{ trashedIssues { id } }").await;
    assert!(d["trashedIssues"].as_array().unwrap().is_empty());
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        &format!("mutation {{ issueDelete(id: \"{id}\") {{ success }} }}"),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    // Normale Listen enthalten es nicht mehr, der Papierkorb schon (workspaceweit und je Team).
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ all: trashedIssues { id title trashed } eng: trashedIssues(team: \"ENG\") { id } web: trashedIssues(team: \"WEB\") { id } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["all"].as_array().unwrap().len(), 1);
    assert_eq!(d["all"][0]["id"], id.as_str());
    assert_eq!(d["all"][0]["trashed"], true);
    assert_eq!(d["eng"].as_array().unwrap().len(), 1);
    assert!(d["web"].as_array().unwrap().is_empty());
    // Gast (kein Mitglied von ENG) sieht den Eintrag nicht.
    let (d, _) = run(&schema, SeedUser::Carol, "{ trashedIssues { id } }").await;
    assert!(d["trashedIssues"].as_array().unwrap().is_empty());
    // Wiederherstellen.
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        &format!("mutation {{ issueUnarchive(id: \"{id}\") {{ success }} }}"),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let (d, _) = run(&schema, SeedUser::Alice, "{ trashedIssues { id } }").await;
    assert!(d["trashedIssues"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn trashed_projects_and_initiatives_restore(pool: PgPool) {
    let schema = setup(pool).await;
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ project(id: \"P-ENG-1\") { id } initiative(id: \"I-1\") { id } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let pid = d["project"]["id"].as_str().unwrap().to_owned();
    let iid = d["initiative"]["id"].as_str().unwrap().to_owned();
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        &format!(
            "mutation {{ projectDelete(id: \"{pid}\") {{ success }} initiativeDelete(id: \"{iid}\") {{ success }} }}"
        ),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ trashedProjects { id name } trashedInitiatives { id name } projects { id } initiatives { id } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["trashedProjects"][0]["id"], pid.as_str());
    assert_eq!(d["trashedInitiatives"][0]["id"], iid.as_str());
    assert!(
        !d["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == pid.as_str())
    );
    assert!(d["initiatives"].as_array().unwrap().is_empty());
    // Ein Gast sieht weder Projekte (ohne Teamzugriff) noch Initiativen im Papierkorb.
    let (d, e) = run(&schema, SeedUser::Carol, "{ trashedProjects { id } }").await;
    assert!(e.is_empty(), "{e:?}");
    assert!(d["trashedProjects"].as_array().unwrap().is_empty());
    let (_, e) = run(&schema, SeedUser::Carol, "{ trashedInitiatives { id } }").await;
    assert!(!e.is_empty());
    // Der Gast kann nicht wiederherstellen.
    let (_, e) = run(
        &schema,
        SeedUser::Carol,
        &format!("mutation {{ initiativeDelete(id: \"{iid}\", restore: true) {{ success }} }}"),
    )
    .await;
    assert!(!e.is_empty());
    // Wiederherstellen (Initiative erst per UUID; Kennung findet nur Aktive).
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        &format!(
            "mutation {{ projectDelete(id: \"{pid}\", restore: true) {{ success }} initiativeDelete(id: \"{iid}\", restore: true) {{ success }} }}"
        ),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let (d, _) = run(
        &schema,
        SeedUser::Alice,
        "{ trashedProjects { id } trashedInitiatives { id } initiatives { id } }",
    )
    .await;
    assert!(d["trashedProjects"].as_array().unwrap().is_empty());
    assert!(d["trashedInitiatives"].as_array().unwrap().is_empty());
    assert_eq!(d["initiatives"].as_array().unwrap().len(), 1);
}
