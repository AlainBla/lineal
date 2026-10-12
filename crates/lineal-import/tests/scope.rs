#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP28: Import einzelner Teams/Projekte, Ziel = bestehendes Lineal-Team, erweiterter Dry-Run-Bericht.
//! Gegen die Fake-Quelle `fixtures::scope_workspace` (erfundene Daten, kein Zugriff auf Linear).

mod common;

use common::*;
use lineal_app::ErrorKind;
use lineal_app::usecases::import::ImportStartInput;
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::team::CreateTeamInput;
use lineal_import::engine::{EngineConfig, RunOutcome};
use lineal_import::{FakeSource, fixtures};
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

const PER_TEAM: i64 = 300;

fn source() -> FakeSource {
    FakeSource::from_json(&fixtures::scope_workspace(PER_TEAM)).unwrap()
}

async fn existing_team(e: &Env, key: &str) -> Uuid {
    e.app
        .create_team(
            &e.alice_ctx(),
            CreateTeamInput {
                name: format!("Bestehendes Team {key}"),
                key: Some(key.into()),
                inherit_workflow_statuses: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .team
        .id
        .0
}

async fn report_messages(e: &Env, run: Uuid, severity: &str) -> Vec<String> {
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), run, Some(severity), 5000, 0)
        .await
        .unwrap();
    entries.into_iter().map(|r| r.message).collect()
}

// ---------------------------------------------------------------------------------------------
// Zwei Teams in einem Lauf (neue Teams)
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn two_teams_in_one_run_keep_statuses_labels_and_shared_projects_exact(pool: PgPool) {
    let e = env(pool).await;
    let src = source();
    let (run, out) = e.import_all(&src, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    let r = e.app.import_get_run(&e.alice_ctx(), run).await.unwrap();
    assert_eq!(r.status, "completed", "{:?}", r.error);

    // Projekte genau einmal, Mehrteam-Projekte mit beiden Teams.
    assert_eq!(e.count("SELECT count(*) FROM project").await, 20);
    assert_eq!(
        e.count("SELECT count(*) FROM project_team").await,
        5 * 2 + 15
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM project p WHERE (SELECT count(*) FROM project_team pt WHERE pt.project_id = p.id) = 2"
        )
        .await,
        5
    );
    // Lead-Team ist das erste Team der Quelle (KOL).
    assert_eq!(
        e.count("SELECT count(*) FROM project p JOIN team t ON t.id = p.lead_team_id WHERE t.key = 'KOL' AND p.name <= 'Projekt 04'").await,
        5
    );
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 2 * PER_TEAM);
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE identifier = 'KOL-300'")
            .await,
        1
    );
    // Statusliste je Team exakt (13 und 11, mehrere je Kategorie, Name/Typ/Farbe/Position).
    assert_eq!(
        e.count(
            "SELECT count(*) FROM workflow_state s JOIN team t ON t.id = s.team_id WHERE t.key = 'KOL'"
        )
        .await,
        13
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM workflow_state s JOIN team t ON t.id = s.team_id WHERE t.key = 'PLN'"
        )
        .await,
        11
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM workflow_state s JOIN team t ON t.id = s.team_id \
             WHERE t.key = 'KOL' AND s.type = 'completed'"
        )
        .await,
        3
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM workflow_state s JOIN team t ON t.id = s.team_id \
             WHERE t.key = 'KOL' AND s.name = 'Blockiert' AND s.type = 'started' \
             AND s.color = '#EB5757' AND s.position = 1.5 * 7 + 0.25"
        )
        .await,
        1
    );
    // Duplicate wird zum Typ `duplicate` (Name ohne Beachtung der Schreibweise).
    assert_eq!(
        e.count("SELECT count(*) FROM workflow_state WHERE type = 'duplicate'")
            .await,
        2
    );
    // 18 Workspace-Labels (kein Team), eines zurückgezogen, Beschreibungen erhalten.
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE team_id IS NULL")
            .await,
        18
    );
    assert_eq!(e.count("SELECT count(*) FROM issue_label").await, 18);
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE retired_at IS NOT NULL")
            .await,
        1
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE description LIKE 'Label %'")
            .await,
        18
    );
    // Agent-Delegierung und unbekannte Enum-Werte brechen nichts; Hinweise stehen im Bericht.
    let warnings = report_messages(&e, run, "warning").await;
    assert!(
        warnings.iter().any(|m| m.contains("delegated to agents")),
        "{warnings:?}"
    );
    assert_eq!(
        e.count("SELECT count(*) FROM project WHERE health IS NULL AND name = 'Projekt 03'")
            .await,
        1
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE sla_type IS NOT NULL")
            .await,
        0
    );
    let errors = report_messages(&e, run, "error").await;
    assert!(errors.is_empty(), "{errors:?}");
    // Relationen über die Teams hinweg bleiben echte Relationen.
    assert!(
        e.count("SELECT count(*) FROM issue_relation r JOIN issue a ON a.id = r.issue_id JOIN issue b ON b.id = r.related_issue_id WHERE a.team_id <> b.team_id")
            .await
            >= 1
    );
}

// ---------------------------------------------------------------------------------------------
// Projektfilter
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn project_filter_imports_only_the_chosen_projects_and_what_they_need(pool: PgPool) {
    let e = env(pool).await;
    let src = source();
    let planned = e.scan(&src).await;
    // Der Scan nennt je Team die Projekte mit Status und Issue-Anzahl.
    let teams = planned.stats["scan"]["teams"].as_array().unwrap();
    let kol = teams.iter().find(|t| t["key"] == "KOL").unwrap();
    let pln = teams.iter().find(|t| t["key"] == "PLN").unwrap();
    assert_eq!(kol["projects"].as_array().unwrap().len(), 13);
    assert_eq!(pln["projects"].as_array().unwrap().len(), 12);
    let p0 = kol["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "p-0")
        .unwrap();
    assert_eq!(p0["name"], "Projekt 00");
    assert_eq!(p0["status"], "Backlog");
    assert!(p0["issueCount"].as_i64().unwrap() > 5);

    // Unbekannte Projekte werden abgelehnt.
    let err = e
        .app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: planned.id,
                projects: vec!["p-unknown".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);

    e.start(
        planned.id,
        ImportStartInput {
            projects: vec!["p-0".into(), "p-5".into()],
            ..Default::default()
        },
    )
    .await;
    let out = e
        .engine(&src, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM project").await, 2);
    // Nur Issues der beiden Projekte.
    let in_projects = src
        .entity(lineal_import::SourceEntity::Issues)
        .iter()
        .filter(|i| i["project"]["id"] == "p-0" || i["project"]["id"] == "p-5")
        .count() as i64;
    assert!(in_projects > 0);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, in_projects);
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE project_id IS NULL")
            .await,
        0
    );
    // Meilensteine/Updates der gewählten Projekte; Kommentare nur zu importierten Issues.
    assert_eq!(
        e.count("SELECT count(*) FROM project_milestone").await,
        1,
        "only p-0 has a milestone"
    );
    assert_eq!(e.count("SELECT count(*) FROM project_update").await, 2);
    assert!(
        e.count("SELECT count(*) FROM comment c WHERE NOT EXISTS (SELECT 1 FROM issue i WHERE i.id = c.issue_id)")
            .await
            == 0
    );
    // Nur beteiligte Personen: die Unbeteiligte (gina) und die Agent-Person fehlen.
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\" WHERE email = 'gina@partner.test'")
            .await,
        0
    );
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\" WHERE email = 'erin@partner.test'")
            .await,
        1
    );
    // Relationen zu nicht importierten Issues: Link-Anhang statt Relation, Warnung im Bericht.
    assert!(
        e.count("SELECT count(*) FROM attachment WHERE title LIKE 'Linear: blocks %' OR title LIKE 'Linear: blocked by %'")
            .await
            > 0
    );
    let warnings = report_messages(&e, planned.id, "warning").await;
    assert!(
        warnings
            .iter()
            .any(|m| m.contains("is not part of this import")),
        "{warnings:?}"
    );
    let r = e
        .app
        .import_get_run(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert!(r.stats["plan"]["relationsOutside"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------------------------------------
// Ziel = bestehendes Lineal-Team
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn import_into_an_existing_team_maps_statuses_and_labels_and_renumbers(pool: PgPool) {
    let e = env(pool).await;
    let team = existing_team(&e, "KP").await;
    // Vorhandene Issues im Zielteam und ein gleichnamiges Workspace-Label.
    let ctx = e.alice_ctx();
    for t in ["Vorhanden A", "Vorhanden B"] {
        e.app
            .create_issue(
                &ctx,
                CreateIssueInput {
                    team: "KP".into(),
                    title: Some(t.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO issue_label (id, organization_id, name, color) VALUES (gen_random_uuid(), $1, 'bug', '#112233')")
        .bind(e.org.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let states_before = e
        .count("SELECT count(*) FROM workflow_state WHERE team_id IS NOT NULL")
        .await;
    let default_before: Option<Uuid> =
        sqlx::query_scalar("SELECT default_issue_state_id FROM team WHERE id = $1")
            .bind(team)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    let teams_before = e.count("SELECT count(*) FROM team").await;

    let src = source();
    let planned = e.scan(&src).await;
    let input = ImportStartInput {
        teams: vec!["KOL".into()],
        projects: vec!["p-5".into(), "p-6".into()],
        target_teams: BTreeMap::from([("KOL".to_owned(), team)]),
        dry_run: true,
        ..Default::default()
    };
    // Probelauf: Plan mit Zuordnungen, nichts wird geschrieben.
    e.start(planned.id, input.clone()).await;
    let out = e
        .engine(&src, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::DryRunFinished);
    let dry = e
        .app
        .import_get_run(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert_eq!(dry.status, "planned");
    let plan = &dry.stats["plan"];
    assert!(plan["statesMapped"].as_u64().unwrap() == 2, "{plan}");
    assert!(plan["statesNew"].as_u64().unwrap() >= 5, "{plan}");
    assert!(plan["labelsNew"].as_u64().unwrap() >= 1, "{plan}");
    assert_eq!(plan["teamsMapped"], 1);
    assert!(plan["usersPlaceholder"].as_u64().unwrap() >= 1, "{plan}");
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 2);

    // Echter Lauf.
    e.start(
        planned.id,
        ImportStartInput {
            dry_run: false,
            ..input.clone()
        },
    )
    .await;
    let out = e
        .engine(&src, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    let run = e
        .app
        .import_get_run(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert_eq!(run.status, "completed", "{:?}", run.error);
    // Kein neues Team; Einstellungen des Zielteams unverändert.
    assert_eq!(e.count("SELECT count(*) FROM team").await, teams_before);
    let default_after: Option<Uuid> =
        sqlx::query_scalar("SELECT default_issue_state_id FROM team WHERE id = $1")
            .bind(team)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(default_before, default_after);
    // Status: vorhandene (Backlog/Duplicate) zugeordnet, fehlende angelegt.
    assert_eq!(
        e.count("SELECT count(*) FROM workflow_state WHERE name = 'Backlog'")
            .await,
        1,
        "Backlog was mapped, not duplicated"
    );
    assert_eq!(
        e.count("SELECT count(*) FROM workflow_state WHERE type = 'duplicate'")
            .await,
        1,
        "Duplicate was mapped to the existing status"
    );
    assert!(
        e.count("SELECT count(*) FROM workflow_state WHERE name = 'Blockiert'")
            .await
            == 1
    );
    assert!(
        e.count("SELECT count(*) FROM workflow_state WHERE team_id IS NOT NULL")
            .await
            > states_before
    );
    // Labels: `Bug` auf das vorhandene Workspace-Label (Name ohne Beachtung der Schreibweise),
    // die übrigen als Workspace-Labels angelegt (die Quelle hat keine Team-Labels).
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE lower(name) = 'bug'")
            .await,
        1
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE team_id IS NOT NULL")
            .await,
        0
    );
    // Neue Nummern ab 3, keine Kollision, Linear-Kennung als Alias und Anhang.
    let in_projects = src
        .entity(lineal_import::SourceEntity::Issues)
        .iter()
        .filter(|i| {
            i["team"]["id"] == "t-kol"
                && (i["project"]["id"] == "p-5" || i["project"]["id"] == "p-6")
        })
        .count() as i64;
    assert!(in_projects > 10);
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE team_id IS NOT NULL")
            .await,
        2 + in_projects
    );
    assert_eq!(
        e.count("SELECT min(number) FROM issue WHERE title LIKE 'Aufgabe %'")
            .await,
        3
    );
    assert_eq!(
        e.count("SELECT max(number) FROM issue").await,
        2 + in_projects
    );
    assert_eq!(
        e.count("SELECT last_issue_number FROM team WHERE key = 'KP'")
            .await,
        2 + in_projects
    );
    // Die Linear-Kennung löst auf (Alias), und der Anhang trägt den Link.
    let linear_ident: String = sqlx::query_scalar(
        "SELECT identifier FROM issue_identifier_alias ORDER BY identifier LIMIT 1",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert!(linear_ident.starts_with("KOL-"));
    let found = e.app.get_issue(&ctx, &linear_ident).await.unwrap();
    assert!(found.view.issue.identifier.starts_with("KP-"));
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM attachment WHERE title = 'Linear: {linear_ident}' AND url LIKE 'https://linear.app/%'"
        ))
        .await,
        1
    );
    // Mitgliedschaften des Zielteams bleiben unverändert; Zyklen mit neuen Nummern.
    assert_eq!(
        e.count("SELECT count(*) FROM team_membership WHERE team_id IS NOT NULL")
            .await,
        e.count("SELECT count(*) FROM team_membership tm JOIN team t ON t.id = tm.team_id WHERE t.key = 'KP'")
            .await
    );
    assert_eq!(e.count("SELECT count(*) FROM cycle").await, 3);

    // Delta: Änderung einer Aufgabe in der Quelle; keine neuen Nummern/Doppel.
    e.clock.set(t0() + chrono::Duration::hours(2));
    assert!(src.patch(
        lineal_import::SourceEntity::Issues,
        "i-kol-5",
        &serde_json::json!({ "title": "Neu betitelt", "updatedAt": "2026-10-10T09:00:00.000Z" }),
    ));
    let planned2 = e.scan(&src).await;
    e.start(
        planned2.id,
        ImportStartInput {
            delta: true,
            dry_run: false,
            ..input.clone()
        },
    )
    .await;
    let out = e
        .engine(&src, EngineConfig::default())
        .run(e.org, planned2.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM issue").await,
        2 + in_projects,
        "delta creates no duplicates"
    );
    assert_eq!(
        e.count("SELECT max(number) FROM issue").await,
        2 + in_projects
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE title = 'Neu betitelt'")
            .await,
        1
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn target_team_needs_visibility_and_write_access(pool: PgPool) {
    let e = env(pool).await;
    let src = source();
    let planned = e.scan(&src).await;
    // Unbekanntes Team: not_found.
    let err = e
        .app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: planned.id,
                teams: vec!["KOL".into()],
                target_teams: BTreeMap::from([("KOL".to_owned(), Uuid::new_v4())]),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    // Ein Admin, der das private Team nicht sehen darf, bekommt not_found (nichts über das Team verraten).
    let secret = e
        .app
        .create_team(
            &e.alice_ctx(),
            CreateTeamInput {
                name: "Geheim".into(),
                key: Some("GEH".into()),
                visibility: Some(lineal_domain::enums::TeamVisibility::Private),
                inherit_workflow_statuses: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .team
        .id
        .0;
    let carl = lineal_domain::ids::UserId(Uuid::new_v4());
    sqlx::query("INSERT INTO \"user\" (id, organization_id, name, display_name, email, role) VALUES ($1, $2, 'Carl', 'carl', 'carl@acme.test', 'admin')")
        .bind(carl.0)
        .bind(e.org.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let err = e
        .app
        .import_start(
            &e.ctx(carl, lineal_domain::enums::UserRole::Admin),
            ImportStartInput {
                run_id: planned.id,
                teams: vec!["KOL".into()],
                target_teams: BTreeMap::from([("KOL".to_owned(), secret)]),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind, ErrorKind::NotFound | ErrorKind::Forbidden),
        "{err:?}"
    );
    // Ein Linear-Team, das nicht importiert wird, kann kein Ziel haben.
    let own = existing_team(&e, "OWN").await;
    let err = e
        .app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: planned.id,
                teams: vec!["KOL".into()],
                target_teams: BTreeMap::from([("PLN".to_owned(), own)]),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 0);
}
