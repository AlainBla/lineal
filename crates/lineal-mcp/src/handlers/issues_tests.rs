//! Contract-Tests für `issues`, `labels` und `sharing` gegen echte Datenbank (Seed `acme`).

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

const DEFAULT_KEYS: [&str; 31] = [
    "id",
    "uuid",
    "title",
    "description",
    "priority",
    "url",
    "gitBranchName",
    "createdAt",
    "updatedAt",
    "archivedAt",
    "completedAt",
    "startedAt",
    "canceledAt",
    "dueDate",
    "slaStartedAt",
    "slaMediumRiskAt",
    "slaHighRiskAt",
    "slaBreachesAt",
    "status",
    "statusType",
    "labels",
    "createdBy",
    "createdById",
    "assignee",
    "assigneeId",
    "project",
    "projectId",
    "parentId",
    "team",
    "teamId",
    "cycleId",
];

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn save_and_get_issue_shape(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let created = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "team": "ENG", "title": "Login schlägt fehl", "priority": 2, "assignee": "me",
                "labels": ["Bug"], "description": "Details @bob" }),
    )
    .await
    .unwrap();
    assert_eq!(created["id"], "ENG-1");
    assert_eq!(created["priority"], json!({ "value": 2, "name": "High" }));
    assert_eq!(created["labels"], json!(["Bug"]));
    assert_eq!(created["assignee"], "Alice Example");
    assert_eq!(created["status"], "Backlog");
    assert_eq!(created["statusType"], "backlog");
    assert_eq!(created["team"], "Engineering");
    assert!(
        created["url"]
            .as_str()
            .unwrap()
            .contains("/acme/issue/ENG-1/")
    );
    assert_eq!(
        created["gitBranchName"].as_str().unwrap().split('/').next(),
        Some("alice")
    );
    for k in DEFAULT_KEYS {
        assert!(created.get(k).is_some(), "Schlüssel {k} fehlt");
    }
    assert!(created.get("relations").is_none());

    let got = call(&app, SeedUser::Alice, "get_issue", json!({ "id": "eng-1" }))
        .await
        .unwrap();
    assert_eq!(got["uuid"], created["uuid"]);
    for k in ["attachments", "documents", "stateHistory"] {
        assert!(got[k].is_array());
    }
    assert!(got.get("relations").is_none());
    let with_rel = call(
        &app,
        SeedUser::Alice,
        "get_issue",
        json!({ "id": "ENG-1", "includeRelations": true }),
    )
    .await
    .unwrap();
    assert_eq!(with_rel["relations"]["duplicateOf"], Value::Null);
    assert_eq!(with_rel["relations"]["blocks"], json!([]));
    let sel = call(
        &app,
        SeedUser::Alice,
        "get_issue",
        json!({ "id": "ENG-1", "fields": ["title", "estimate", "relations"] }),
    )
    .await
    .unwrap();
    let keys: Vec<_> = sel.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["id", "title", "estimate", "relations"]);
    assert_eq!(sel["estimate"], Value::Null);
    let e = call(
        &app,
        SeedUser::Alice,
        "get_issue",
        json!({ "id": "P-ENG-1" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let e = call(
        &app,
        SeedUser::Alice,
        "get_issue",
        json!({ "id": "ENG-99" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    for args in [
        json!({ "title": "x" }),
        json!({ "team": "ENG" }),
        json!({ "team": "ENG", "title": "x", "patch": [{"op":"append","text":"y"}] }),
    ] {
        let e = call(&app, SeedUser::Alice, "save_issue", args)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_issue_is_not_found_via_mcp(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let des = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "team": "DES", "title": "Logo" }),
    )
    .await
    .unwrap();
    for id in ["DES-1", des["uuid"].as_str().unwrap()] {
        let e = call(&app, SeedUser::Alice, "get_issue", json!({ "id": id }))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound);
        let e = call(
            &app,
            SeedUser::Alice,
            "save_issue",
            json!({ "id": id, "title": "x" }),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound);
    }
    let list = call(&app, SeedUser::Alice, "list_issues", json!({}))
        .await
        .unwrap();
    assert_eq!(list["issues"], json!([]));
    let e = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "team": "DES" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "team": "DES", "title": "x" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn update_semantics_patch_relations_and_team_change(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    for t in ["Eins", "Zwei"] {
        call(
            &app,
            SeedUser::Bob,
            "save_issue",
            json!({ "team": "WEB", "title": t, "description": "alpha beta alpha", "assignee": "bob" }),
        )
        .await
        .unwrap();
    }
    let e = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "patch": [{ "op": "replace", "old_string": "alpha", "new_string": "x" }] }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert_eq!(e.details["matches"], 2);
    let got = call(&app, SeedUser::Bob, "get_issue", json!({ "id": "WEB-1" }))
        .await
        .unwrap();
    assert_eq!(got["description"], "alpha beta alpha");
    let e = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "description": "a", "patch": [{ "op": "append", "text": "b" }] }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let r = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "assignee": null }),
    )
    .await
    .unwrap();
    assert_eq!(r["assignee"], Value::Null);
    let r = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-2", "title": "Zwei!" }),
    )
    .await
    .unwrap();
    assert_eq!(r["assignee"], "Bob Example");
    let r = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "blocks": ["WEB-2"], "relatedTo": ["WEB-2"] }),
    )
    .await
    .unwrap();
    assert_eq!(r["relations"]["blocks"][0]["id"], "WEB-2");
    let dup = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-2", "duplicateOf": "WEB-1" }),
    )
    .await
    .unwrap();
    assert_eq!(dup["statusType"], "duplicate");
    assert_eq!(dup["relations"]["duplicateOf"]["id"], "WEB-1");
    let back = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-2", "duplicateOf": null }),
    )
    .await
    .unwrap();
    assert_eq!(back["statusType"], "backlog");
    let e = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "team": "DES", "addLabels": ["Bug"] }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let moved = call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "id": "WEB-1", "team": "DES" }),
    )
    .await
    .unwrap();
    assert_eq!(moved["id"], "DES-1");
    assert_eq!(moved["previousIdentifiers"], json!(["WEB-1"]));
    let old = call(&app, SeedUser::Bob, "get_issue", json!({ "id": "WEB-1" }))
        .await
        .unwrap();
    assert_eq!(old["id"], "DES-1");
    for args in [
        json!({ "id": "WEB-2", "priority": 9 }),
        json!({ "id": "WEB-2", "state": "Nirgends" }),
        json!({ "id": "WEB-2", "relatedTo": ["WEB-2"] }),
    ] {
        assert!(call(&app, SeedUser::Bob, "save_issue", args).await.is_err());
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn list_issues_pagination_and_filters(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    for (i, p) in [1, 2, 3].iter().enumerate() {
        call(
            &app,
            SeedUser::Alice,
            "save_issue",
            json!({ "team": "ENG", "title": format!("I{i}"), "priority": p }),
        )
        .await
        .unwrap();
    }
    let p1 = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "limit": 2, "orderBy": "createdAt" }),
    )
    .await
    .unwrap();
    assert_eq!(p1["issues"].as_array().unwrap().len(), 2);
    assert_eq!(p1["hasNextPage"], true);
    let cursor = p1["cursor"].as_str().unwrap().to_string();
    let p2 = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "limit": 2, "orderBy": "createdAt", "cursor": cursor }),
    )
    .await
    .unwrap();
    assert_eq!(p2["issues"].as_array().unwrap().len(), 1);
    assert_eq!(p2["hasNextPage"], false);
    assert!(p2.get("cursor").is_none());
    let e = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "limit": 2, "orderBy": "createdAt", "cursor": cursor, "priority": 1 }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let only = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "priority": 2, "fields": ["title", "labels"] }),
    )
    .await
    .unwrap();
    assert_eq!(only["issues"].as_array().unwrap().len(), 1);
    assert_eq!(only["issues"][0]["labels"], json!([]));
    let keys: Vec<_> = only["issues"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, ["id", "title", "labels"]);
    let none = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "assignee": null }),
    )
    .await
    .unwrap();
    assert_eq!(none["issues"].as_array().unwrap().len(), 3);
    assert!(
        call(
            &app,
            SeedUser::Alice,
            "list_issues",
            json!({ "updatedAt": "-P1D" })
        )
        .await
        .is_ok()
    );
    for args in [
        json!({ "label": "Nope" }),
        json!({ "state": "Nope" }),
        json!({ "team": "Nope" }),
        json!({ "assignee": "ghost" }),
    ] {
        assert_eq!(
            call(&app, SeedUser::Alice, "list_issues", args)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }
    let e = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "query": "x" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn label_tools(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let g = call(
        &app,
        SeedUser::Alice,
        "save_issue_label",
        json!({ "name": "Platform", "isGroup": true }),
    )
    .await
    .unwrap();
    assert_eq!(g["isGroup"], true);
    assert_eq!(g["groupType"], "singleSelect");
    let c = call(
        &app,
        SeedUser::Alice,
        "save_issue_label",
        json!({ "name": "iOS", "parent": "Platform", "color": "#112233" }),
    )
    .await
    .unwrap();
    assert_eq!(c["parent"]["name"], "Platform");
    let t = call(
        &app,
        SeedUser::Bob,
        "save_issue_label",
        json!({ "name": "Frontend", "teamId": "WEB" }),
    )
    .await
    .unwrap();
    assert_eq!(t["team"]["key"], "WEB");
    let list = call(
        &app,
        SeedUser::Alice,
        "list_issue_labels",
        json!({ "team": "WEB", "includeGroups": true }),
    )
    .await
    .unwrap();
    assert!(list["labels"].as_array().unwrap().len() >= 6);
    assert_eq!(list["hasNextPage"], false);
    let r = call(
        &app,
        SeedUser::Alice,
        "retire_issue_label",
        json!({ "id": "Platform" }),
    )
    .await
    .unwrap();
    assert_eq!(r["affectedChildren"].as_array().unwrap().len(), 1);
    assert!(r["retiredAt"].is_string());
    let r = call(
        &app,
        SeedUser::Alice,
        "restore_issue_label",
        json!({ "id": "Platform" }),
    )
    .await
    .unwrap();
    assert_eq!(r["retiredAt"], Value::Null);
    let u = call(
        &app,
        SeedUser::Alice,
        "save_issue_label",
        json!({ "id": "iOS", "description": "Apple" }),
    )
    .await
    .unwrap();
    assert_eq!(u["description"], "Apple");
    let e = call(
        &app,
        SeedUser::Bob,
        "save_issue_label",
        json!({ "id": "Frontend", "teamId": "DES" }),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        e.kind,
        ErrorKind::InvalidRequest | ErrorKind::NotFound
    ));
    let e = call(
        &app,
        SeedUser::Alice,
        "save_issue_label",
        json!({ "name": "Bug" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    let p = call(
        &app,
        SeedUser::Alice,
        "save_project_label",
        json!({ "name": "Strategic" }),
    )
    .await
    .unwrap();
    assert_eq!(p["team"], Value::Null);
    let lp = call(&app, SeedUser::Alice, "list_project_labels", json!({}))
        .await
        .unwrap();
    assert_eq!(lp["labels"].as_array().unwrap().len(), 1);
    call(
        &app,
        SeedUser::Alice,
        "retire_project_label",
        json!({ "id": "Strategic" }),
    )
    .await
    .unwrap();
    call(
        &app,
        SeedUser::Alice,
        "restore_project_label",
        json!({ "id": "Strategic" }),
    )
    .await
    .unwrap();
    call(
        &app,
        SeedUser::Alice,
        "save_initiative_label",
        json!({ "name": "Q4" }),
    )
    .await
    .unwrap();
    let li = call(
        &app,
        SeedUser::Alice,
        "list_initiative_labels",
        json!({ "name": "q" }),
    )
    .await
    .unwrap();
    assert_eq!(li["labels"].as_array().unwrap().len(), 1);
    call(
        &app,
        SeedUser::Alice,
        "retire_initiative_label",
        json!({ "id": "Q4" }),
    )
    .await
    .unwrap();
    call(
        &app,
        SeedUser::Alice,
        "restore_initiative_label",
        json!({ "id": "Q4" }),
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sharing_tools(pool: sqlx::PgPool) {
    let app = setup(pool.clone()).await;
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb",
    )
    .execute(&pool)
    .await
    .unwrap();
    call(
        &app,
        SeedUser::Bob,
        "save_issue",
        json!({ "team": "DES", "title": "Intern" }),
    )
    .await
    .unwrap();
    for user in ["alice", "me"] {
        let e = call(
            &app,
            SeedUser::Bob,
            "share_issue",
            json!({ "issue": "DES-1", "user": user }),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
    }
    let s = call(
        &app,
        SeedUser::Bob,
        "share_issue",
        json!({ "issue": "DES-1", "user": "alice@acme.example" }),
    )
    .await
    .unwrap();
    assert_eq!(s["issue"], "DES-1");
    assert_eq!(s["shared"], true);
    assert!(s["sharedAt"].is_string());
    assert_eq!(
        call(&app, SeedUser::Alice, "get_issue", json!({ "id": "DES-1" }))
            .await
            .unwrap()["title"],
        "Intern"
    );
    let again = call(
        &app,
        SeedUser::Bob,
        "share_issue",
        json!({ "issue": "DES-1", "user": "bob@acme.example" }),
    )
    .await
    .unwrap();
    assert_eq!(again["shared"], false);
    assert_eq!(again["message"], "User already has access");
    let u = call(
        &app,
        SeedUser::Bob,
        "unshare_issue",
        json!({ "issue": "DES-1", "user": "alice@acme.example" }),
    )
    .await
    .unwrap();
    assert_eq!(u["shared"], false);
    let e = call(&app, SeedUser::Alice, "get_issue", json!({ "id": "DES-1" }))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = call(
        &app,
        SeedUser::Alice,
        "share_issue",
        json!({ "issue": "DES-1", "user": "carol@acme.example" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}
