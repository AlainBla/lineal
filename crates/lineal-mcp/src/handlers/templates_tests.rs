//! Contract-Tests für `templates` sowie `template`/`links` in `save_issue` und `save_project`.

use crate::dispatch::dispatch;
use lineal_app::ports::Store;
use lineal_app::usecases::template::SaveTemplateInput;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::enums::TemplateType;
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

async fn make(app: &App, ty: TemplateType, name: &str, team: Option<&str>, data: Value) {
    app.save_template(
        &ctx_for(SeedUser::Alice),
        SaveTemplateInput {
            template_type: Some(ty),
            name: Some(name.into()),
            team: team.map(str::to_owned),
            template_data: Some(data),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn list_and_get_template_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    make(
        &app,
        TemplateType::Issue,
        "Bug Report",
        Some("ENG"),
        json!({ "title": "[Bug] ", "description": "## Schritte\n{{placeholder: ...}}", "priority": 2,
                "subIssues": [ { "title": "Regressionstest ergänzen" } ] }),
    )
    .await;
    make(
        &app,
        TemplateType::Document,
        "PRD",
        None,
        json!({ "title": "PRD" }),
    )
    .await;

    let all = call(&app, SeedUser::Alice, "list_templates", json!({}))
        .await
        .unwrap();
    assert_eq!(all["hasNextPage"], false);
    assert!(all.get("cursor").is_none());
    assert_eq!(all["templates"].as_array().unwrap().len(), 2);
    let web = call(
        &app,
        SeedUser::Alice,
        "list_templates",
        json!({ "team": "WEB" }),
    )
    .await
    .unwrap();
    assert_eq!(web["templates"].as_array().unwrap().len(), 2); // geerbt von ENG + Workspace
    let docs = call(
        &app,
        SeedUser::Alice,
        "list_templates",
        json!({ "type": "document" }),
    )
    .await
    .unwrap();
    assert_eq!(docs["templates"][0]["name"], "PRD");
    assert!(docs["templates"][0]["team"].is_null());
    for key in [
        "id",
        "name",
        "type",
        "description",
        "team",
        "isForm",
        "updatedAt",
    ] {
        assert!(docs["templates"][0].get(key).is_some(), "{key}");
    }

    let t = call(
        &app,
        SeedUser::Alice,
        "get_template",
        json!({ "id": "Bug Report" }),
    )
    .await
    .unwrap();
    assert_eq!(t["team"]["key"], "ENG");
    assert_eq!(t["content"]["priority"], 2);
    assert!(
        t["content"]["description"]
            .as_str()
            .unwrap()
            .contains("{{placeholder")
    );
    assert_eq!(t["subIssues"][0]["title"], "Regressionstest ergänzen");
    assert_eq!(t["subIssues"][0]["labelIds"], json!([]));
    assert_eq!(t["isForm"], false);
    let miss = call(
        &app,
        SeedUser::Alice,
        "get_template",
        json!({ "id": "Nope" }),
    )
    .await;
    let e = miss.err().unwrap();
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(e.message.contains("list_templates"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn save_issue_with_template_and_links(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    make(
        &app,
        TemplateType::Issue,
        "Neues Feature",
        Some("WEB"),
        json!({ "title": "Feature: ", "priority": 3, "stateType": "backlog",
                "subIssues": [ { "title": "Design" }, { "title": "Umsetzung" }, { "title": "Tests" } ] }),
    )
    .await;
    let out = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({
            "team": "WEB", "template": "Neues Feature", "title": "Dark Mode",
            "links": [ { "url": "https://figma.example/file/abc", "title": "Entwürfe" } ]
        }),
    )
    .await
    .unwrap();
    assert_eq!(out["title"], "Dark Mode");
    let subs = out["subIssues"].as_array().unwrap();
    assert_eq!(subs.len(), 3);
    let root_n: i32 = out["id"]
        .as_str()
        .unwrap()
        .strip_prefix("WEB-")
        .unwrap()
        .parse()
        .unwrap();
    for (i, s) in subs.iter().enumerate() {
        assert_eq!(s["id"], format!("WEB-{}", root_n + 1 + i as i32));
    }
    assert_eq!(out["warnings"], json!([]));
    // Link wurde angehängt.
    let linked = app
        .list_issue_attachments(&ctx_for(SeedUser::Alice), out["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].url, "https://figma.example/file/abc");

    // Template beim Update und ungültige Links -> invalid_request.
    let upd = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "id": out["id"], "template": "Neues Feature" }),
    )
    .await;
    assert_eq!(upd.err().unwrap().kind, ErrorKind::InvalidRequest);
    let bad = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "team": "WEB", "title": "X", "links": [ { "url": "javascript:alert(1)" } ] }),
    )
    .await;
    assert_eq!(bad.err().unwrap().kind, ErrorKind::InvalidRequest);
    // Links an einem bestehenden Issue (Update).
    let more = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "id": out["id"], "links": [ { "url": "https://docs.example/spec", "title": "Spec" } ] }),
    )
    .await
    .unwrap();
    assert_eq!(more["id"], out["id"]);
    let linked = app
        .list_issue_attachments(&ctx_for(SeedUser::Alice), out["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(linked.len(), 2);
    // Nicht sichtbares Template (anderes privates Team) -> not_found.
    make(
        &app,
        TemplateType::Issue,
        "Intern",
        Some("ENG"),
        json!({ "title": "E" }),
    )
    .await;
    let nf = call(
        &app,
        SeedUser::Alice,
        "save_issue",
        json!({ "team": "DES", "template": "Intern" }),
    )
    .await;
    assert_eq!(nf.err().unwrap().kind, ErrorKind::NotFound);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn save_project_with_template(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    make(
        &app,
        TemplateType::Project,
        "Launch",
        Some("ENG"),
        json!({ "name": "Launch", "priority": 2, "milestones": [ { "name": "Alpha" } ],
                "issues": [ { "milestone": "Alpha", "issue": { "title": "Kickoff" } } ] }),
    )
    .await;
    let p = call(
        &app,
        SeedUser::Alice,
        "save_project",
        json!({ "name": "Launch Q4", "template": "Launch", "addTeams": ["ENG"] }),
    )
    .await
    .unwrap();
    assert_eq!(p["name"], "Launch Q4");
    let issues = call(
        &app,
        SeedUser::Alice,
        "list_issues",
        json!({ "project": p["id"] }),
    )
    .await
    .unwrap();
    assert_eq!(issues["issues"][0]["title"], "Kickoff");
    let upd = call(
        &app,
        SeedUser::Alice,
        "save_project",
        json!({ "id": p["id"], "template": "Launch" }),
    )
    .await;
    assert_eq!(upd.err().unwrap().kind, ErrorKind::InvalidRequest);
}
