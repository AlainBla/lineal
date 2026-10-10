//! Contract-Tests für `documents` und `search_documentation` gegen echte Datenbank (Seed `acme`).

use crate::dispatch::dispatch;
use lineal_app::ports::Store;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup(pool: sqlx::PgPool) -> Arc<App> {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

async fn call(
    app: &App,
    who: SeedUser,
    tool: &str,
    args: Value,
) -> Result<Value, lineal_app::AppError> {
    dispatch(app, &ctx_for(who), tool, args)
        .await
        .expect("known tool")
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn save_get_list_document_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let doc = call(
        &app,
        SeedUser::Alice,
        "save_document",
        json!({ "title": "API Rate Limits", "content": "x".repeat(700), "team": "ENG", "icon": "Rocket" }),
    )
    .await
    .unwrap();
    for key in [
        "id",
        "slugId",
        "title",
        "content",
        "icon",
        "color",
        "url",
        "createdAt",
        "updatedAt",
        "archivedAt",
        "creator",
        "updatedBy",
        "project",
        "initiative",
        "team",
        "issue",
        "cycle",
    ] {
        assert!(doc.get(key).is_some(), "missing {key}");
    }
    assert_eq!(doc["team"]["key"], "ENG");
    assert!(doc["project"].is_null() && doc["issue"].is_null());
    assert!(
        doc["url"]
            .as_str()
            .unwrap()
            .contains("/acme/document/api-rate-limits-")
    );
    assert_eq!(doc["content"].as_str().unwrap().len(), 700);

    // get_document: UUID, slugId, URL; Liste kürzt den Inhalt.
    for r in [doc["id"].clone(), doc["slugId"].clone(), doc["url"].clone()] {
        let got = call(&app, SeedUser::Bob, "get_document", json!({ "id": r }))
            .await
            .unwrap();
        assert_eq!(got["id"], doc["id"]);
    }
    let list = call(
        &app,
        SeedUser::Bob,
        "list_documents",
        json!({ "query": "xxx" }),
    )
    .await
    .unwrap();
    assert_eq!(list["hasNextPage"], false);
    let content = list["documents"][0]["content"].as_str().unwrap();
    assert!(content.contains("truncated, use `get_document`"));
    let fields = call(
        &app,
        SeedUser::Bob,
        "list_documents",
        json!({ "fields": ["title", "team"] }),
    )
    .await
    .unwrap();
    assert_eq!(
        fields["documents"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["id", "title", "team"]
    );

    // Patch und Fehlerfälle.
    let upd = call(
        &app,
        SeedUser::Alice,
        "save_document",
        json!({ "id": doc["id"], "patch": [{ "op": "append", "text": "!" }] }),
    )
    .await
    .unwrap();
    assert!(upd["content"].as_str().unwrap().ends_with("x!"));
    let two = call(
        &app,
        SeedUser::Alice,
        "save_document",
        json!({ "title": "X", "team": "ENG", "project": "P-ENG-1" }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(two.kind, ErrorKind::InvalidRequest);
    assert!(two.message.contains("exactly one"));
    let miss = call(
        &app,
        SeedUser::Alice,
        "get_document",
        json!({ "id": "000000000000" }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(miss.kind, ErrorKind::NotFound);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_documents_hidden_via_mcp(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let doc = call(
        &app,
        SeedUser::Bob,
        "save_document",
        json!({ "title": "Intern", "content": "geheim-token", "team": "DES" }),
    )
    .await
    .unwrap();
    let get = call(
        &app,
        SeedUser::Alice,
        "get_document",
        json!({ "id": doc["id"] }),
    )
    .await;
    assert_eq!(get.err().unwrap().kind, ErrorKind::NotFound);
    let list = call(&app, SeedUser::Alice, "list_documents", json!({}))
        .await
        .unwrap();
    assert_eq!(list["documents"].as_array().unwrap().len(), 0);
    let search = call(
        &app,
        SeedUser::Alice,
        "list_documents",
        json!({ "query": "geheim-token" }),
    )
    .await
    .unwrap();
    assert_eq!(search["documents"].as_array().unwrap().len(), 0);
    let by_team = call(
        &app,
        SeedUser::Alice,
        "list_documents",
        json!({ "teamId": "DES" }),
    )
    .await;
    assert_eq!(by_team.err().unwrap().kind, ErrorKind::NotFound);
    let bob = call(
        &app,
        SeedUser::Bob,
        "list_documents",
        json!({ "query": "geheim-token" }),
    )
    .await
    .unwrap();
    assert_eq!(bob["documents"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn search_documentation_returns_paged_hits(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let r = call(
        &app,
        SeedUser::Alice,
        "search_documentation",
        json!({ "query": "Triage" }),
    )
    .await
    .unwrap();
    let results = r["results"].as_array().unwrap();
    assert!(!results.is_empty() && results.len() <= 5);
    for hit in results {
        assert!(hit["url"].as_str().unwrap().contains("/docs/"));
        assert!(!hit["excerpt"].as_str().unwrap().is_empty());
        assert!(hit["title"].is_string());
    }
    assert_eq!(r["page"], 0);
    assert_eq!(r["hasNextPage"], true);
    let p2 = call(
        &app,
        SeedUser::Alice,
        "search_documentation",
        json!({ "query": "Triage", "page": 1 }),
    )
    .await
    .unwrap();
    assert_ne!(p2["results"][0]["url"], results[0]["url"]);
    let none = call(
        &app,
        SeedUser::Alice,
        "search_documentation",
        json!({ "query": "zzzqqq-nicht-vorhanden" }),
    )
    .await
    .unwrap();
    assert_eq!(none["results"].as_array().unwrap().len(), 0);
    assert_eq!(none["hasNextPage"], false);
    let empty = call(
        &app,
        SeedUser::Alice,
        "search_documentation",
        json!({ "query": "  " }),
    )
    .await;
    assert_eq!(empty.err().unwrap().kind, ErrorKind::InvalidRequest);
}
