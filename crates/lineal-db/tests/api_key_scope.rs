#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP17 P1.6: API-Keys mit Team-Beschränkung (`api_key.team_ids`) sehen und ändern nur diese Teams.

use lineal_app::ports::Store;
use lineal_app::usecases::auth::{CreateApiKeyInput, hash_token};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::team::ListTeamsInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: PgPool) -> (App, SeedData) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (
        Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap(),
        data,
    )
}

/// Löst einen Key wie die Adapter über den Token-Lookup zu einem `RequestCtx` auf.
async fn ctx_of_key(app: &App, key: &str) -> RequestCtx {
    let mut tx = app.store.begin_system().await.unwrap();
    let p = tx
        .auth()
        .find_principal_by_token_hash(&hash_token(key))
        .await
        .unwrap()
        .expect("key resolves");
    RequestCtx {
        org_id: p.org_id,
        actor: p.actor,
        request_id: "req_key".into(),
        scopes: p.scopes,
        team_scope: p.team_scope,
        via_session: false,
    }
}

async fn make_key(app: &App, teams: Option<Vec<&str>>) -> String {
    app.create_api_key(
        &ctx_for(SeedUser::Alice),
        CreateApiKeyInput {
            label: "ci".into(),
            scopes: vec!["read".into(), "write".into()],
            teams: teams.map(|t| t.into_iter().map(String::from).collect()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .key
}

fn new_issue(team: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some("Aus dem Key".into()),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn team_restricted_key_sees_and_changes_only_its_teams(pool: PgPool) {
    let (app, data) = setup(pool).await;
    let restricted = ctx_of_key(&app, &make_key(&app, Some(vec!["ENG"])).await).await;
    assert_eq!(restricted.team_scope, Some(vec![data.eng]));

    // Lesen: nur ENG (Alice sieht ohne Beschränkung auch WEB).
    let teams = app
        .list_teams(&restricted, ListTeamsInput::default())
        .await
        .unwrap();
    let keys: Vec<_> = teams.items.iter().map(|t| t.team.key.clone()).collect();
    assert_eq!(keys, ["ENG"]);
    app.get_team(&restricted, "ENG").await.unwrap();
    assert_eq!(
        app.get_team(&restricted, "WEB").await.unwrap_err().kind,
        ErrorKind::NotFound,
        "Teams außerhalb des Scopes sind nicht auffindbar"
    );

    // Schreiben: im Scope ok, außerhalb not_found (wie ein unsichtbares Team).
    app.create_issue(&restricted, new_issue("ENG"))
        .await
        .unwrap();
    assert_eq!(
        app.create_issue(&restricted, new_issue("WEB"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unrestricted_and_empty_scopes_behave_as_documented(pool: PgPool) {
    let (app, _data) = setup(pool).await;
    // Ohne Beschränkung: alles, was Alice sieht.
    let open = ctx_of_key(&app, &make_key(&app, None).await).await;
    assert_eq!(open.team_scope, None);
    let keys: Vec<_> = app
        .list_teams(&open, ListTeamsInput::default())
        .await
        .unwrap()
        .items
        .iter()
        .map(|t| t.team.key.clone())
        .collect();
    assert!(keys.contains(&"ENG".to_string()) && keys.contains(&"WEB".to_string()));
    // Leere Liste = kein Team (sicherer Default).
    let none = ctx_of_key(&app, &make_key(&app, Some(vec![])).await).await;
    assert!(
        app.list_teams(&none, ListTeamsInput::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}
