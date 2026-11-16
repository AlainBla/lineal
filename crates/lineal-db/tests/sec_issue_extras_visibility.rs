#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-63 N2: Ein reiner Freigabe-Empfänger sieht ein Issue, darf aber Team- und Projektnamen
//! des (für ihn unsichtbaren) privaten Teams nicht erfahren. Seed `acme`, privates Team DES
//! (Bob Mitglied, Alice nicht).

use chrono::{TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::project::NewProject;
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::ids::ProjectId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const DES_PROJECT: Uuid = Uuid::from_u128(0x9201_EC70_0000_7000_8000_0000_0000_00D5);

fn create(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

async fn setup(pool: PgPool) -> (Arc<App>, SeedData) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    // Privates Projekt im Team DES (Lead DES) – für Alice unsichtbar.
    let now = Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap();
    let number = tx.teams().next_project_number(data.des).await.unwrap();
    tx.projects()
        .insert(&NewProject {
            id: ProjectId(DES_PROJECT),
            name: "GeheimProjekt".into(),
            lead_team_id: data.des,
            number,
            description: None,
            content: None,
            color: None,
            icon: None,
            // Workspace-Projektstatus „In Progress“ aus dem Seed.
            status_id: Uuid::from_u128(0x9A05_0000_0000_7000_8000_0000_0000_0002),
            priority: 0,
            sort_order: 1000.0,
            lead_id: Some(data.bob),
            creator_id: Some(data.bob),
            start_date: None,
            start_date_resolution: None,
            target_date: None,
            target_date_resolution: None,
            started_at: Some(now),
            completed_at: None,
            canceled_at: None,
            now,
        })
        .await
        .unwrap();
    tx.projects()
        .add_team(ProjectId(DES_PROJECT), data.des, 0.0, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // Freigaben (issueSharingEnabled) sind im Seed aus; für den Empfänger-Pfad einschalten.
    sqlx::query("UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb WHERE id = $1")
        .bind(data.org.0)
        .execute(&pool)
        .await
        .unwrap();
    (test_app(Arc::new(store)), data)
}

/// Ein reiner Freigabe-Empfänger (Alice) sieht ein DES-Issue, darf aber Team- und Projektnamen
/// des privaten Teams nicht erfahren.
#[sqlx::test(migrations = "./migrations")]
async fn extras_hide_team_and_project_from_share_recipient(pool: PgPool) {
    let (app, data) = setup(pool).await;
    let (alice, bob): (RequestCtx, RequestCtx) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));

    let issue = app
        .create_issue(
            &bob,
            CreateIssueInput {
                project: Some(DES_PROJECT.to_string()),
                ..create("DES", "Geheimes Issue")
            },
        )
        .await
        .unwrap();
    app.share_issue(&bob, &issue.issue.id.to_string(), &data.alice.0.to_string())
        .await
        .unwrap();

    // Bob (Mitglied) sieht Team- und Projektnamen.
    let bob_view = app
        .get_issue(&bob, &issue.issue.id.to_string())
        .await
        .unwrap();
    assert_eq!(bob_view.view.extras.team_name, "Design");
    assert_eq!(
        bob_view.view.extras.project_name.as_deref(),
        Some("GeheimProjekt")
    );

    // Alice sieht das Issue über die Freigabe, aber nicht die Namen des privaten Teams/Projekts.
    let alice_view = app
        .get_issue(&alice, &issue.issue.id.to_string())
        .await
        .unwrap();
    assert!(
        alice_view.view.extras.team_name.is_empty(),
        "Teamname eines unsichtbaren Teams darf nicht durchsickern (N2): {:?}",
        alice_view.view.extras.team_name
    );
    assert_eq!(
        alice_view.view.extras.project_name, None,
        "Projektname eines unsichtbaren Projekts darf nicht durchsickern (N2)"
    );
    assert_eq!(
        alice_view.view.extras.project_identifier, None,
        "Projektkennung eines unsichtbaren Projekts darf nicht durchsickern (N2)"
    );
}
