//! Contract-Tests für die Gruppen `workspace` und `views` sowie `list_issues` mit `query` und
//! `customView` gegen echte Datenbank (Seed `acme`).

use crate::dispatch::dispatch;
use lineal_app::ErrorKind;
use lineal_app::ports::Store;
use lineal_app::usecases::overview::OverviewInput;
use lineal_app::usecases::view::CreateViewInput;
use lineal_app::{App, AppError};
use lineal_db::PgStore;
use lineal_domain::ids::TeamId;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: Arc<App>,
    pool: PgPool,
    d: SeedData,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Env {
        app: test_app(Arc::new(store)),
        pool,
        d: SeedData::acme(),
    }
}

async fn call(env: &Env, who: SeedUser, tool: &str, args: Value) -> Result<Value, AppError> {
    dispatch(&env.app, &ctx_for(who), tool, args)
        .await
        .expect("known tool")
}

impl Env {
    async fn issue(&self, team: TeamId, slot: u128, number: i32, title: &str, extra: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(id)
        .bind(self.d.org.0)
        .bind(team.0)
        .bind(number)
        .bind(title)
        .bind(seed_workflow_state_ids(slot)[1].0)
        .bind(self.d.alice.0)
        .execute(&self.pool)
        .await
        .unwrap();
        if !extra.is_empty() {
            sqlx::query(&format!("UPDATE issue SET {extra} WHERE id = $1"))
                .bind(id)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        id
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn overview_tool_matches_the_use_case_numbers_and_respects_visibility(pool: PgPool) {
    let env = setup(pool).await;
    env.issue(env.d.eng, 1, 101, "Eng a", "due_date = '2000-01-01'")
        .await;
    env.issue(env.d.eng, 1, 102, "Eng b", "").await;
    env.issue(env.d.web, 2, 101, "Web a", "").await;
    env.issue(env.d.des, 3, 101, "Secret", "due_date = '2000-01-01'")
        .await;

    // Kriterium 4: dieselben Zahlen wie der Use-Case (= GraphQL `workspaceOverview`).
    for who in [SeedUser::Alice, SeedUser::Bob] {
        let out = call(&env, who, "get_workspace_overview", json!({}))
            .await
            .unwrap();
        let direct = env
            .app
            .workspace_overview(&ctx_for(who), OverviewInput::default())
            .await
            .unwrap()
            .to_json();
        assert_eq!(out, direct, "{who:?}");
    }
    let alice = call(&env, SeedUser::Alice, "get_workspace_overview", json!({}))
        .await
        .unwrap();
    assert_eq!(alice["totals"]["openIssues"], 3);
    assert_eq!(alice["totals"]["overdue"], 1);
    let keys: Vec<&str> = alice["teams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["ENG", "WEB"]);
    let bob = call(&env, SeedUser::Bob, "get_workspace_overview", json!({}))
        .await
        .unwrap();
    assert_eq!(bob["totals"]["openIssues"], 4);
    // includeSubTeams: Default an, ausschaltbar.
    let eng = |v: &Value| v["teams"][0]["counts"]["openIssues"].as_i64().unwrap();
    assert_eq!(eng(&alice), 3);
    let flat = call(
        &env,
        SeedUser::Alice,
        "get_workspace_overview",
        json!({"includeSubTeams": false}),
    )
    .await
    .unwrap();
    assert_eq!(eng(&flat), 2);
    // Teams-Filter und Periode.
    let web = call(
        &env,
        SeedUser::Alice,
        "get_workspace_overview",
        json!({"teams": ["WEB"], "period": "30d"}),
    )
    .await
    .unwrap();
    assert_eq!(web["period"], "30d");
    assert_eq!(web["teams"].as_array().unwrap().len(), 1);
    // Fehler: privates Team → not_found, Gast → forbidden, ungültige Periode → invalid_request.
    assert_eq!(
        call(
            &env,
            SeedUser::Alice,
            "get_workspace_overview",
            json!({"teams": ["DES"]})
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        call(&env, SeedUser::Carol, "get_workspace_overview", json!({}))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        call(
            &env,
            SeedUser::Alice,
            "get_workspace_overview",
            json!({"period": "1y"})
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn list_custom_views_returns_visible_metadata_only(pool: PgPool) {
    let env = setup(pool).await;
    let mk = |name: &str, shared: bool, team: Option<&str>| CreateViewInput {
        name: name.into(),
        description: Some("d".into()),
        icon: None,
        color: None,
        model: lineal_app::ports::repos::view::ViewModel::Issue,
        filter: json!({"priority": {"in": [1]}}),
        shared,
        team: team.map(str::to_owned),
        project: None,
        initiative: None,
        preferences: None,
    };
    let bob = ctx_for(SeedUser::Bob);
    env.app
        .create_view(&bob, mk("Bobs private", false, None))
        .await
        .unwrap();
    env.app
        .create_view(&bob, mk("Web board", true, Some("WEB")))
        .await
        .unwrap();
    env.app
        .create_view(&bob, mk("Design board", true, Some("DES")))
        .await
        .unwrap();
    env.app
        .create_view(&ctx_for(SeedUser::Alice), mk("Company", true, None))
        .await
        .unwrap();

    let names = |v: &Value| -> Vec<String> {
        let mut n: Vec<String> = v["customViews"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap().to_owned())
            .collect();
        n.sort();
        n
    };
    let a = call(&env, SeedUser::Alice, "list_custom_views", json!({}))
        .await
        .unwrap();
    assert_eq!(
        names(&a),
        ["Company", "Web board"],
        "no private or private-team views for Alice"
    );
    let b = call(&env, SeedUser::Bob, "list_custom_views", json!({}))
        .await
        .unwrap();
    assert_eq!(
        names(&b),
        ["Bobs private", "Company", "Design board", "Web board"]
    );
    // Metadaten, keine Filterdefinition.
    let v = &b["customViews"][0];
    assert!(v.get("filter").is_none() && v.get("filterData").is_none());
    assert_eq!(v["modelName"], "Issue");
    assert!(v["url"].as_str().unwrap().contains("/view/"));
    let web = b["customViews"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "Web board")
        .unwrap();
    assert_eq!(web["team"]["key"], "WEB");
    assert_eq!(web["owner"]["name"], "Bob Example");
    assert_eq!(web["shared"], true);
    // Filter und Paging.
    let shared = call(
        &env,
        SeedUser::Bob,
        "list_custom_views",
        json!({"shared": false}),
    )
    .await
    .unwrap();
    assert_eq!(names(&shared), ["Bobs private"]);
    let team = call(
        &env,
        SeedUser::Bob,
        "list_custom_views",
        json!({"team": "WEB"}),
    )
    .await
    .unwrap();
    assert_eq!(names(&team), ["Web board"]);
    let q = call(
        &env,
        SeedUser::Bob,
        "list_custom_views",
        json!({"query": "board"}),
    )
    .await
    .unwrap();
    assert_eq!(names(&q), ["Design board", "Web board"]);
    let p1 = call(
        &env,
        SeedUser::Bob,
        "list_custom_views",
        json!({"limit": 3}),
    )
    .await
    .unwrap();
    assert_eq!(p1["hasNextPage"], true);
    let p2 = call(
        &env,
        SeedUser::Bob,
        "list_custom_views",
        json!({"limit": 3, "cursor": p1["cursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(p2["customViews"].as_array().unwrap().len(), 1);
    assert_eq!(p2["hasNextPage"], false);
    // Team-Filter auf ein privates Team → not_found für Alice.
    assert_eq!(
        call(
            &env,
            SeedUser::Alice,
            "list_custom_views",
            json!({"team": "DES"})
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn list_issues_query_and_custom_view_parameters(pool: PgPool) {
    let env = setup(pool).await;
    env.issue(env.d.eng, 1, 101, "Login fails", "priority = 1")
        .await;
    env.issue(env.d.eng, 1, 102, "Login slow", "priority = 2")
        .await;
    env.issue(env.d.des, 3, 101, "Secret login plan", "").await;
    let ids = |v: &Value| -> Vec<String> {
        let mut n: Vec<String> = v["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_owned())
            .collect();
        n.sort();
        n
    };
    // query: Volltext, Kennung; privates Team unsichtbar.
    let r = call(
        &env,
        SeedUser::Alice,
        "list_issues",
        json!({"query": "login"}),
    )
    .await
    .unwrap();
    assert_eq!(ids(&r), ["ENG-101", "ENG-102"]);
    let r = call(
        &env,
        SeedUser::Bob,
        "list_issues",
        json!({"query": "login"}),
    )
    .await
    .unwrap();
    assert_eq!(ids(&r).len(), 3);
    let r = call(
        &env,
        SeedUser::Alice,
        "list_issues",
        json!({"query": "DES-101"}),
    )
    .await
    .unwrap();
    assert!(ids(&r).is_empty());
    let r = call(
        &env,
        SeedUser::Alice,
        "list_issues",
        json!({"query": "eng-102"}),
    )
    .await
    .unwrap();
    assert_eq!(ids(&r), ["ENG-102"]);
    // customView: View UND Zusatzparameter.
    let bob = ctx_for(SeedUser::Bob);
    env.app
        .create_view(
            &bob,
            CreateViewInput {
                name: "Urgent".into(),
                description: None,
                icon: None,
                color: None,
                model: lineal_app::ports::repos::view::ViewModel::Issue,
                filter: json!({"priority": {"in": [1, 2]}}),
                shared: false,
                team: None,
                project: None,
                initiative: None,
                preferences: None,
            },
        )
        .await
        .unwrap();
    let r = call(
        &env,
        SeedUser::Bob,
        "list_issues",
        json!({"customView": "Urgent"}),
    )
    .await
    .unwrap();
    assert_eq!(ids(&r), ["ENG-101", "ENG-102"]);
    let r = call(
        &env,
        SeedUser::Bob,
        "list_issues",
        json!({"customView": "Urgent", "priority": 2}),
    )
    .await
    .unwrap();
    assert_eq!(ids(&r), ["ENG-102"]);
    // Fremde persönliche View → not_found; query + customView → invalid_request.
    assert_eq!(
        call(
            &env,
            SeedUser::Alice,
            "list_issues",
            json!({"customView": "Urgent"})
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        call(
            &env,
            SeedUser::Bob,
            "list_issues",
            json!({"customView": "Urgent", "query": "x"})
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
}
