#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_arguments,
    dead_code,
    unused_imports
)]
//! Paket 5 (Testlauf-Bugs): Planung und Settings gegen echte Datenbank.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lineal_app::ports::repos::user::{PageCursor, PageRequest};
use lineal_app::ports::{Clock, IdGen, Store};
use lineal_app::usecases::cycle::{
    CycleKind, CycleSettingsInput, ExistingActiveIssues, UpdateCycleInput,
};
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::usecases::initiative::{ListInitiativesInput, SaveInitiativeInput};
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_app::usecases::project::{ListProjectsInput, SaveProjectInput, ViewOpts};
use lineal_app::usecases::team::{CreateTeamInput, UpdateTeamInput};
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{DateResolution, Health, TeamVisibility};
use lineal_testkit::seed::planning::{seed_cycle_id, seed_initiative_id, seed_project_id};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct At(DateTime<Utc>);
impl Clock for At {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}
struct V7;
impl IdGen for V7 {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, mi, 0).single().unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn app_at(pool: &PgPool, t: DateTime<Utc>) -> App {
    let mut app = Arc::try_unwrap(test_app(Arc::new(PgStore::new(pool.clone()))))
        .ok()
        .unwrap();
    app.clock = Arc::new(At(t));
    app.ids = Arc::new(V7);
    app
}

/// Seed `acme`; Uhr = 2026-10-10T07:15Z (wie die Cycle-Fenster des Seeds).
async fn setup(pool: PgPool) -> (App, SeedData, PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (app_at(&pool, utc(2026, 10, 10, 7, 15)), data, pool)
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn teams_input(teams: &[&str]) -> SaveProjectInput {
    SaveProjectInput {
        name: Some("Relaunch".into()),
        add_teams: teams.iter().map(|t| (*t).to_owned()).collect(),
        ..Default::default()
    }
}

async fn new_team(app: &App, ctx: &RequestCtx, name: &str, key: &str, tz: &str) -> Uuid {
    app.create_team(
        ctx,
        CreateTeamInput {
            name: name.into(),
            key: Some(key.into()),
            timezone: Some(tz.into()),
            visibility: Some(TeamVisibility::Public),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .team
    .id
    .0
}

async fn state_id(pool: &PgPool, team: Uuid, kind: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM workflow_state WHERE team_id = $1 AND type = $2 ORDER BY position LIMIT 1",
    )
    .bind(team)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn issue(
    pool: &PgPool,
    org: Uuid,
    team: Uuid,
    number: i32,
    state_kind: &str,
    cycle: Option<Uuid>,
    project: Option<Uuid>,
) -> Uuid {
    let state = state_id(pool, team, state_kind).await;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, cycle_id, project_id) \
         VALUES ($1, $2, $3, $4, 'Testissue', $5, $6, $7)",
    )
    .bind(id)
    .bind(org)
    .bind(team)
    .bind(number)
    .bind(state)
    .bind(cycle)
    .bind(project)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_01_shift_all_rejects_out_of_range_days_without_panic(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let c3 = seed_cycle_id(3).to_string();
    for days in [
        2_000_000_000_i64,
        -2_000_000_000,
        2_147_483_647,
        100_000,
        3661,
        -3661,
    ] {
        let e = app.shift_cycles(&alice, &c3, days).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "days={days}");
        assert_eq!(e.details["field"], "daysToShift");
    }
    app.shift_cycles(&alice, &c3, 3660).await.unwrap();
}

async fn windows(pool: &PgPool, team: Uuid) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    sqlx::query_as("SELECT starts_at, ends_at FROM cycle WHERE team_id = $1 ORDER BY starts_at")
        .bind(team)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_03_timezone_change_with_gapless_cycles_recomputes_future(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // Fingerprint des Zeitplans setzen (Seed legt keinen an).
    app.configure_team_cycles(&alice, "ENG", CycleSettingsInput::default())
        .await
        .unwrap();
    for tz in ["Asia/Kolkata", "America/Los_Angeles", "Pacific/Auckland"] {
        app.update_team(
            &alice,
            "ENG",
            UpdateTeamInput {
                timezone: Some(tz.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{tz}: {e:?}"));
        let w = windows(&pool, data.eng.0).await;
        for pair in w.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "{tz}: Überlappung {pair:?}");
            assert!(pair[1].0 < pair[1].1);
        }
        // Aktiver Cycle (#2) bleibt unverändert.
        assert_eq!(w[1], (utc(2026, 10, 4, 22, 0), utc(2026, 10, 18, 22, 0)));
    }
}

/// B5.04 / Sicherheitsprüfung: dieselbe Rechteprüfung auf allen Schreibpfaden eines Projekts mit
/// für den Aufrufer unsichtbarem Team; nichts über das unsichtbare Team in Antworten/Fehlern.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_04_all_project_write_paths_share_one_rights_rule(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    let shared = app
        .save_project(&bob, teams_input(&["ENG", "DES"]))
        .await
        .unwrap();
    let pid = shared.identifier.clone();
    let ms = app
        .save_milestone(
            &bob,
            SaveMilestoneInput {
                project: pid.clone(),
                name: Some("M1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .1
        .milestone
        .id
        .0
        .to_string();
    let upd = app
        .post_project_update(&bob, &pid, "hi", None, false)
        .await
        .unwrap();
    let upd_id = upd.update.id.0.to_string();

    // Erwartung je Akteur: Some(true) = nicht not_found/forbidden, None = not_found.
    // Pfad -> Ergebnis-Art je Akteur (Alice, Bob, Carol).
    macro_rules! check {
        ($name:expr, $actor:expr, $expect_ok:expr, $call:expr) => {{
            let r = $call;
            match ($expect_ok, r) {
                (true, Ok(_)) => {}
                (true, Err(e)) => {
                    panic!("{} als {}: unerwartet {:?}", $name, stringify!($actor), e)
                }
                (false, Ok(_)) => panic!(
                    "{} als {}: hätte scheitern müssen",
                    $name,
                    stringify!($actor)
                ),
                (false, Err(e)) => {
                    assert_eq!(
                        e.kind,
                        ErrorKind::NotFound,
                        "{} als {}",
                        $name,
                        stringify!($actor)
                    );
                    assert!(!e.message.contains("DES"), "{}: {}", $name, e.message);
                    assert!(!format!("{:?}", e.details).contains("DES"), "{}", $name);
                }
            }
        }};
    }
    for (actor_name, ctx, ok) in [
        ("alice", &alice, true),
        ("bob", &bob, true),
        ("carol", &carol, false),
    ] {
        check!(
            "projectUpdate",
            ctx,
            ok,
            app.save_project(
                ctx,
                SaveProjectInput {
                    id: Some(pid.clone()),
                    name: Some("N".into()),
                    ..Default::default()
                }
            )
            .await
        );
        check!(
            "setTeams",
            ctx,
            ok,
            app.save_project(
                ctx,
                SaveProjectInput {
                    id: Some(pid.clone()),
                    set_teams: Some(if actor_name == "bob" {
                        vec!["ENG".into(), "DES".into()]
                    } else {
                        vec!["ENG".into()]
                    }),
                    ..Default::default()
                }
            )
            .await
        );
        check!(
            "milestoneCreate",
            ctx,
            ok,
            app.save_milestone(
                ctx,
                SaveMilestoneInput {
                    project: pid.clone(),
                    name: Some(format!("M2-{actor_name}")),
                    ..Default::default()
                }
            )
            .await
        );
        check!(
            "milestoneUpdate",
            ctx,
            ok,
            app.save_milestone(
                ctx,
                SaveMilestoneInput {
                    project: pid.clone(),
                    id: Some(ms.clone()),
                    name: Some("M1b".into()),
                    ..Default::default()
                }
            )
            .await
        );
        check!(
            "statusUpdateCreate",
            ctx,
            ok,
            app.post_project_update(ctx, &pid, "x", None, false).await
        );
        check!(
            "documentCreate",
            ctx,
            ok,
            app.save_document(
                ctx,
                SaveDocumentInput {
                    title: Some("Doc".into()),
                    content: Some("c".into()),
                    parent: ParentRefs {
                        project: Some(pid.clone()),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            )
            .await
        );
        let del = if ok {
            let m = app
                .save_milestone(
                    &bob,
                    SaveMilestoneInput {
                        project: pid.clone(),
                        name: Some(format!("tmp-{actor_name}")),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .1
                .milestone
                .id
                .0
                .to_string();
            app.delete_milestone(ctx, &pid, &m).await
        } else {
            app.delete_milestone(ctx, &pid, &ms).await
        };
        check!("milestoneDelete", ctx, ok, del);
        // Lesen: Hülle sichtbar für Alice/Bob; Carol not_found; kein DES in der Sicht.
        match app.get_project(ctx, &pid, ViewOpts::default()).await {
            Ok(v) => {
                assert!(ok);
                if std::ptr::eq(ctx, &alice) {
                    assert!(v.teams.iter().all(|t| t.key != "DES"));
                }
            }
            Err(e) => {
                assert!(!ok);
                assert_eq!(e.kind, ErrorKind::NotFound);
            }
        }
    }
    // Edit/Delete eines Updates: Autor Bob ok, Alice (nicht Autor) nicht not_found sondern forbidden,
    // Carol not_found.
    assert_eq!(
        kind(
            app.edit_project_update(&carol, &upd_id, Some("z"), None)
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(app.delete_project_update(&carol, &upd_id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            app.edit_project_update(&alice, &upd_id, Some("z"), None)
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_project_archived(&carol, &pid, true).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(app.set_project_trashed(&carol, &pid, true).await),
        ErrorKind::NotFound
    );
    // Alice darf weder DES entfernen noch das Lead-Team von DES wegwechseln.
    assert_eq!(
        kind(
            app.save_project(
                &alice,
                SaveProjectInput {
                    id: Some(pid.clone()),
                    remove_teams: vec!["DES".into()],
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // DES bleibt nach allen Schreibzugriffen am Projekt.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM project_team WHERE project_id = $1")
        .bind(shared.project.id.0)
        .fetch_one(&_pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_09_lock_to_active_and_auto_assign_apply_on_create(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    app.configure_team_cycles(
        &alice,
        "ENG",
        CycleSettingsInput {
            lock_to_active: Some(true),
            existing_active_issues: Some(ExistingActiveIssues::AddToCycle),
            auto_assign_completed: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mk = |state: &str| CreateIssueInput {
        team: "ENG".into(),
        title: Some("Neu".into()),
        state: Some(state.into()),
        ..Default::default()
    };
    let todo = app.create_issue(&alice, mk("Todo")).await.unwrap();
    assert_eq!(todo.issue.cycle_id, Some(seed_cycle_id(2)));
    let done = app.create_issue(&alice, mk("Done")).await.unwrap();
    assert_eq!(done.issue.cycle_id, Some(seed_cycle_id(2)));
    let backlog = app
        .create_issue(
            &alice,
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("B".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(backlog.issue.cycle_id, None);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_08_b5_10_initiative_projects_are_recursive_and_rollup_respects_staleness(pool: PgPool) {
    let (app, _data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let parent = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                name: Some("Eltern".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let child = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                name: Some("Kind".into()),
                parent_initiatives: vec![parent.identifier.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let project = app
        .save_project(
            &alice,
            SaveProjectInput {
                state: Some("started".into()),
                ..teams_input(&["ENG"])
            },
        )
        .await
        .unwrap();
    app.set_initiative_project(&alice, &child.identifier, &project.identifier, true)
        .await
        .unwrap();
    app.post_project_update(
        &alice,
        &project.identifier,
        "ok",
        Some(Health::OnTrack),
        false,
    )
    .await
    .unwrap();
    let v = app
        .get_initiative(&alice, &parent.identifier)
        .await
        .unwrap();
    assert_eq!(
        v.projects.len(),
        1,
        "Projekte der Sub-Initiative zählen mit"
    );
    assert_eq!(v.health_rollup.on_track, 1);
    // Update 20 Tage alt (Erinnerungszyklus 7 Tage + 3): veraltet -> zählt als none.
    sqlx::query("UPDATE project SET health_updated_at = $2 WHERE id = $1")
        .bind(project.project.id.0)
        .bind(utc(2026, 9, 20, 0, 0))
        .execute(&pool)
        .await
        .unwrap();
    let v = app
        .get_initiative(&alice, &parent.identifier)
        .await
        .unwrap();
    assert_eq!(v.health_rollup.on_track, 0);
    assert_eq!(v.health_rollup.none, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_11_projects_are_paginated_by_cursor(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    for _ in 0..5 {
        app.save_project(&alice, teams_input(&["ENG"]))
            .await
            .unwrap();
    }
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let (page, views) = app
            .list_projects(
                &alice,
                ListProjectsInput {
                    page: PageRequest {
                        limit: 2,
                        cursor,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        pages += 1;
        for v in &views {
            assert!(seen.insert(v.project.id.0), "doppelter Eintrag");
        }
        match page.next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len(), 6, "5 neue plus Seed-Projekt");
    assert_eq!(pages, 3);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_16_22_23_cycle_update_validation(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let c3 = seed_cycle_id(3).to_string();
    let upd = |name: Option<Option<String>>| UpdateCycleInput {
        id: c3.clone(),
        name,
        description: None,
        starts_at: None,
        ends_at: None,
    };
    // Leerer Name = zurücksetzen.
    let v = app
        .update_cycle(&alice, upd(Some(Some("Sprint".into()))))
        .await
        .unwrap();
    assert_eq!(v.cycle.name.as_deref(), Some("Sprint"));
    let v = app
        .update_cycle(&alice, upd(Some(Some("  ".into()))))
        .await
        .unwrap();
    assert_eq!(v.cycle.name, None);
    for bad in ["x".repeat(256), "Sprint 2147483647".to_string()] {
        let e = app
            .update_cycle(&alice, upd(Some(Some(bad))))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(e.details["field"], "name");
    }
    // Mindestdauer ein Tag.
    let e = app
        .update_cycle(
            &alice,
            UpdateCycleInput {
                ends_at: Some(utc(2026, 10, 18, 22, 0) + chrono::Duration::hours(11)),
                starts_at: Some(utc(2026, 10, 19, 0, 0)),
                ..upd(None)
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_27_28_backlog_to_future_cycle_and_done_into_previous(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let issue = app
        .create_issue(
            &alice,
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Spaeter".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(issue.issue.state_type.as_str(), "backlog");
    let v = app
        .update_issue(
            &alice,
            UpdateIssueInput {
                id: issue.issue.identifier.clone(),
                cycle: Some(Some("3".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.state_type.as_str(), "unstarted");
    // Erledigtes Issue darf in den zuletzt abgeschlossenen Cycle (Nr. 1).
    let done = app
        .create_issue(
            &alice,
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Vergessen".into()),
                state: Some("Done".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let v = app
        .update_issue(
            &alice,
            UpdateIssueInput {
                id: done.issue.identifier.clone(),
                cycle: Some(Some("1".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.cycle_id, Some(seed_cycle_id(1)));
    // Ein offenes Issue nicht.
    let e = app
        .update_issue(
            &alice,
            UpdateIssueInput {
                id: issue.issue.identifier.clone(),
                cycle: Some(Some("1".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_29_activation_with_old_start_date_ends_with_an_active_cycle(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    app.configure_team_cycles(
        &alice,
        "WEB",
        CycleSettingsInput {
            cycles_enabled: Some(true),
            duration: Some(1),
            cooldown: Some(0),
            start_day: Some(1),
            upcoming_count: Some(2),
            first_start_date: Some(date(2015, 1, 5)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, current) = app
        .list_cycles(&alice, "WEB", Some(CycleKind::Current))
        .await
        .unwrap();
    assert_eq!(
        current.len(),
        1,
        "ein aktiver Cycle sofort nach Aktivierung"
    );
    let _ = (data, pool);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_19_new_date_without_resolution_resets_the_resolution(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let p = app
        .save_project(
            &alice,
            SaveProjectInput {
                start_date: Some(Some(date(2026, 12, 5))),
                start_date_resolution: Some(Some(DateResolution::Month)),
                ..teams_input(&["ENG"])
            },
        )
        .await
        .unwrap();
    assert_eq!(p.project.start_date, Some(date(2026, 12, 1)));
    let p = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(p.identifier.clone()),
                start_date: Some(Some(date(2026, 12, 15))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p.project.start_date, Some(date(2026, 12, 15)));
    assert_eq!(p.project.start_date_resolution, None);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_31_rich_text_fields_have_length_limits(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let e = app
        .save_project(
            &alice,
            SaveProjectInput {
                description: Some("x".repeat(500_001)),
                ..teams_input(&["ENG"])
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::LimitExceeded);
    let p = app
        .save_project(&alice, teams_input(&["ENG"]))
        .await
        .unwrap();
    let e = app
        .post_project_update(&alice, &p.identifier, &"y".repeat(100_001), None, false)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::LimitExceeded);
    let e = app
        .save_milestone(
            &alice,
            SaveMilestoneInput {
                project: p.identifier.clone(),
                name: Some("M".into()),
                description: Some("z".repeat(100_001)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::LimitExceeded);
}
