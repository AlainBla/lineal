//! Contract-Tests für `teams_users` und `statuses` gegen echte Datenbank (Seed `acme`).

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
async fn workspace_and_teams(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let ws = call(&app, SeedUser::Alice, "get_workspace", json!({}))
        .await
        .unwrap();
    assert_eq!(ws["name"], "Acme");
    assert_eq!(ws["urlKey"], "acme");
    assert!(ws["url"].as_str().unwrap().ends_with("/acme"));

    let list = call(&app, SeedUser::Alice, "list_teams", json!({}))
        .await
        .unwrap();
    let keys: Vec<_> = list["teams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["key"].as_str().unwrap().to_string())
        .collect();
    // Alice (Owner) sieht das private DES als Metadaten (B8.05).
    assert!(
        keys.contains(&"ENG".into())
            && keys.contains(&"WEB".into())
            && keys.contains(&"DES".into())
    );
    assert_eq!(list["hasNextPage"], false);
    assert!(list.get("cursor").is_none());
    let web = list["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["key"] == "WEB")
        .unwrap();
    assert_eq!(web["displayName"], "Engineering › Web");
    assert!(web["parentId"].is_string());

    let kids = call(
        &app,
        SeedUser::Alice,
        "list_teams",
        json!({ "parentTeam": "ENG" }),
    )
    .await
    .unwrap();
    assert_eq!(kids["teams"].as_array().unwrap().len(), 1);
    let desc = call(
        &app,
        SeedUser::Alice,
        "list_teams",
        json!({ "ancestorTeam": "eng", "query": "we" }),
    )
    .await
    .unwrap();
    assert_eq!(desc["teams"][0]["key"], "WEB");
    // Paginierung mit Cursor, an Filter gebunden
    let p1 = call(&app, SeedUser::Bob, "list_teams", json!({ "limit": 1 }))
        .await
        .unwrap();
    assert_eq!(p1["hasNextPage"], true);
    let cur = p1["cursor"].as_str().unwrap().to_string();
    let p2 = call(
        &app,
        SeedUser::Bob,
        "list_teams",
        json!({ "limit": 5, "cursor": cur }),
    )
    .await
    .unwrap();
    assert_eq!(p2["teams"].as_array().unwrap().len(), 2);
    let e = call(
        &app,
        SeedUser::Bob,
        "list_teams",
        json!({ "limit": 5, "cursor": cur, "query": "x" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let e = call(
        &app,
        SeedUser::Alice,
        "list_teams",
        json!({ "parentTeam": "NOPE" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);

    let by_key = call(&app, SeedUser::Alice, "get_team", json!({ "query": "eng" }))
        .await
        .unwrap();
    let id = by_key["id"].as_str().unwrap().to_string();
    for q in [id.as_str(), "Engineering", "ENG"] {
        let t = call(&app, SeedUser::Alice, "get_team", json!({ "query": q }))
            .await
            .unwrap();
        assert_eq!(t["id"], by_key["id"], "{q}");
    }
    assert_eq!(by_key["timezone"], "Europe/Berlin");
    assert_eq!(by_key["issueEstimationType"], "notUsed");
    assert!(
        call(&app, SeedUser::Alice, "get_team", json!({ "query": "DES" }))
            .await
            .is_ok()
    );
    for who in [SeedUser::Carol] {
        let e = call(&app, who, "get_team", json!({ "query": "DES" }))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound);
        assert_eq!(e.details["type"], "Team");
    }
    assert!(
        call(&app, SeedUser::Bob, "get_team", json!({ "query": "DES" }))
            .await
            .is_ok()
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn users(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let me = call(&app, SeedUser::Alice, "get_user", json!({ "query": "me" }))
        .await
        .unwrap();
    assert_eq!(me["displayName"], "alice");
    assert_eq!(me["isAdmin"], true);
    assert_eq!(me["isGuest"], false);
    let teams: Vec<_> = me["teams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["key"].as_str().unwrap())
        .collect();
    assert_eq!(teams, ["ENG"]);
    for q in ["bob", "bob@acme.example", "Bob Example"] {
        let u = call(&app, SeedUser::Alice, "get_user", json!({ "query": q }))
            .await
            .unwrap();
        assert_eq!(u["displayName"], "bob");
    }
    let e = call(
        &app,
        SeedUser::Carol,
        "get_user",
        json!({ "query": "alice" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = call(
        &app,
        SeedUser::Alice,
        "get_user",
        json!({ "query": "nobody@acme.example" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let all = call(&app, SeedUser::Alice, "list_users", json!({}))
        .await
        .unwrap();
    assert_eq!(all["users"].as_array().unwrap().len(), 3);
    let by_team = call(
        &app,
        SeedUser::Alice,
        "list_users",
        json!({ "team": "WEB" }),
    )
    .await
    .unwrap();
    assert_eq!(by_team["users"].as_array().unwrap().len(), 2);
    let q = call(
        &app,
        SeedUser::Alice,
        "list_users",
        json!({ "query": "CAROL" }),
    )
    .await
    .unwrap();
    assert_eq!(q["users"][0]["isGuest"], true);
    let e = call(
        &app,
        SeedUser::Alice,
        "list_users",
        json!({ "team": "DES" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let page = call(&app, SeedUser::Alice, "list_users", json!({ "limit": 2 }))
        .await
        .unwrap();
    assert_eq!(page["hasNextPage"], true);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn statuses(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let list = call(
        &app,
        SeedUser::Bob,
        "list_issue_statuses",
        json!({ "team": "ENG" }),
    )
    .await
    .unwrap();
    let names: Vec<_> = list["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Backlog",
            "Todo",
            "In Progress",
            "Done",
            "Canceled",
            "Duplicate"
        ]
    );
    assert_eq!(list["statuses"][0]["isDefault"], true);
    assert_eq!(list["statuses"][1]["isDefault"], false);
    assert_eq!(list["hasNextPage"], false);
    let web = call(
        &app,
        SeedUser::Bob,
        "list_issue_statuses",
        json!({ "team": "WEB" }),
    )
    .await
    .unwrap();
    assert_eq!(web["statuses"].as_array().unwrap().len(), 6);
    for (who, team) in [
        (SeedUser::Bob, "NOPE"),
        (SeedUser::Alice, "DES"),
        (SeedUser::Carol, "ENG"),
    ] {
        let e = call(&app, who, "list_issue_statuses", json!({ "team": team }))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound, "{team}");
    }
    let id = list["statuses"][2]["id"].as_str().unwrap().to_string();
    let by_id = call(
        &app,
        SeedUser::Bob,
        "get_issue_status",
        json!({ "id": id, "name": "", "team": "ENG" }),
    )
    .await
    .unwrap();
    assert_eq!(by_id["name"], "In Progress");
    assert_eq!(by_id["team"]["key"], "ENG");
    let by_name = call(
        &app,
        SeedUser::Bob,
        "get_issue_status",
        json!({ "id": "", "name": "in progress", "team": "ENG" }),
    )
    .await
    .unwrap();
    assert_eq!(by_name["id"], by_id["id"]);
    let by_cat = call(
        &app,
        SeedUser::Bob,
        "get_issue_status",
        json!({ "id": "", "name": "started", "team": "ENG" }),
    )
    .await
    .unwrap();
    assert_eq!(by_cat["id"], by_id["id"]);
    let e = call(
        &app,
        SeedUser::Bob,
        "get_issue_status",
        json!({ "id": "", "name": "", "team": "ENG" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let e = call(
        &app,
        SeedUser::Bob,
        "get_issue_status",
        json!({ "id": id, "name": "", "team": "WEB" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}
