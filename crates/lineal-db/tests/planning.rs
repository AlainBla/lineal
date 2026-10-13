#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::type_complexity,
    clippy::cloned_ref_to_slice_refs
)]
//! WP07: Planung (Projekte, Meilensteine, Cycles, Initiativen, Status-Updates) gegen echte Datenbank.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::ports::{Clock, IdGen, Store};
use lineal_app::usecases::cycle::{CycleKind, CycleSettingsInput};
use lineal_app::usecases::initiative::{ListInitiativesInput, SaveInitiativeInput};
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_app::usecases::project::{ListProjectsInput, SaveProjectInput, ViewOpts};
use lineal_app::usecases::team::CreateTeamInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{Health, TeamVisibility};
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

// ---------------------------------------------------------------------------------------------
// Projekte
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_with_team_the_creator_is_not_a_member_of(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice); // Owner, Mitglied nur in ENG
    let bob = ctx_for(SeedUser::Bob); // Mitglied in ENG, WEB und DES

    // WEB ist öffentlich: Zugriff genügt, keine Mitgliedschaft nötig (docs/02 §6.4).
    let v = app
        .save_project(&alice, teams_input(&["ENG", "WEB"]))
        .await
        .unwrap();
    assert_eq!(v.identifier, "P-ENG-2", "P-ENG-1 ist das Seed-Projekt");
    assert_eq!(v.teams.len(), 2);
    assert_eq!(v.status.name, "Backlog");

    // DES ist privat und Alice nicht Mitglied: not_found, auch für Admins/Owner.
    let err = app.save_project(&alice, teams_input(&["ENG", "DES"])).await;
    assert_eq!(kind(err), ErrorKind::NotFound);
    let err = app.save_project(&alice, teams_input(&["DES"])).await;
    assert_eq!(kind(err), ErrorKind::NotFound);

    // Bob (Mitglied in beiden) darf; leadTeam Default = erstes Team, Zähler pro Team atomar.
    let v = app
        .save_project(
            &bob,
            SaveProjectInput {
                state: Some("Planned".into()),
                lead: Some(Some("me".into())),
                ..teams_input(&["DES", "ENG"])
            },
        )
        .await
        .unwrap();
    assert_eq!(v.identifier, "P-DES-1");
    assert_eq!(v.lead_team.as_ref().unwrap().key, "DES");
    assert_eq!(v.status.name, "Planned");
    assert_eq!(v.lead.as_ref().unwrap().display_name, "bob");

    // Fehlende Teams: invalid_request.
    assert_eq!(
        kind(
            app.save_project(
                &alice,
                SaveProjectInput {
                    name: Some("X".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_stays_hidden_in_the_project_envelope(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol); // Gast, nur Mitglied in WEB

    // Projekt mit ENG (öffentlich, Lead) und DES (privat) von Bob.
    let shared = app
        .save_project(
            &bob,
            SaveProjectInput {
                lead_team: Some("ENG".into()),
                ..teams_input(&["ENG", "DES"])
            },
        )
        .await
        .unwrap();
    let v = app
        .get_project(&alice, &shared.identifier, ViewOpts::default())
        .await
        .unwrap();
    let keys: Vec<_> = v.teams.iter().map(|t| t.key.clone()).collect();
    assert_eq!(keys, ["ENG"], "DES verschwindet für Nicht-Mitglieder");
    assert!(v.lead_team.is_some());
    assert!(!format!("{:?}", v.teams).contains("Design"));
    // Bob sieht beide Teams.
    let v = app
        .get_project(&bob, &shared.identifier, ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(v.teams.len(), 2);

    // Lead-Team privat: Nicht-Mitglieder sehen leadTeam = null, die Kennung bleibt.
    let dl = app
        .save_project(
            &bob,
            SaveProjectInput {
                lead_team: Some("DES".into()),
                ..teams_input(&["ENG", "DES"])
            },
        )
        .await
        .unwrap();
    let v = app
        .get_project(&alice, &dl.identifier, ViewOpts::default())
        .await
        .unwrap();
    assert!(v.lead_team.is_none());
    assert!(v.identifier.starts_with("P-DES-"));

    // Nur privates Team: unsichtbar (not_found), auch per Name und in Listen.
    let only = app
        .save_project(
            &bob,
            SaveProjectInput {
                name: Some("Geheim".into()),
                ..teams_input(&["DES"])
            },
        )
        .await
        .unwrap();
    assert_eq!(
        kind(
            app.get_project(&alice, &only.identifier, ViewOpts::default())
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(app.get_project(&alice, "Geheim", ViewOpts::default()).await),
        ErrorKind::NotFound
    );
    let (_, list) = app
        .list_projects(
            &alice,
            ListProjectsInput {
                page: PageRequest::default(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(list.iter().all(|p| p.project.name != "Geheim"));
    let (_, list_bob) = app
        .list_projects(
            &bob,
            ListProjectsInput {
                page: PageRequest::default(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(list_bob.iter().any(|p| p.project.name == "Geheim"));

    // Gast Carol (nur WEB): Projekte mit ENG/DES sind für sie nicht auffindbar.
    assert_eq!(
        kind(
            app.get_project(&carol, &shared.identifier, ViewOpts::default())
                .await
        ),
        ErrorKind::NotFound
    );
    // Mit WEB als beteiligtem Team sieht sie die Hülle (nur WEB).
    let web = app
        .save_project(
            &bob,
            SaveProjectInput {
                name: Some("Web".into()),
                ..teams_input(&["WEB", "DES"])
            },
        )
        .await
        .unwrap();
    let v = app
        .get_project(&carol, &web.identifier, ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(
        v.teams.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
        ["WEB"]
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_lifecycle_status_dates_lead_team_and_labels(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let org = data.org.0;

    let v = app
        .save_project(&alice, teams_input(&["ENG"]))
        .await
        .unwrap();
    let id = v.project.id.0;
    assert!(v.project.started_at.is_none());

    // Status Kategorie `started`: startedAt gesetzt; Warnung beim Abschluss.
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(v.identifier.clone()),
                state: Some("started".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.status.name, "In Progress");
    let started = v.project.started_at.expect("startedAt");
    issue(&pool, org, data.eng.0, 1, "started", None, Some(id)).await;
    sqlx::query("INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(org).bind(id).bind(data.eng.0).execute(&pool).await.unwrap();
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                state: Some("Completed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(v.project.completed_at.is_some());
    assert_eq!(
        v.open_issue_count,
        Some(1),
        "Warnung: offene Issues bleiben im Projekt"
    );
    // Wieder öffnen: erster Start bleibt, completedAt leer.
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                state: Some("started".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.project.started_at, Some(started));
    assert!(v.project.completed_at.is_none());
    assert_eq!(
        kind(
            app.save_project(
                &alice,
                SaveProjectInput {
                    id: Some(id.to_string()),
                    state: Some("nope".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );

    // Daten mit Auflösung werden normalisiert; Start nach Ziel und Auflösung ohne Datum sind Fehler.
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                start_date: Some(Some(date(2026, 10, 14))),
                start_date_resolution: Some(Some(lineal_domain::enums::DateResolution::Quarter)),
                target_date: Some(Some(date(2026, 11, 14))),
                target_date_resolution: Some(Some(lineal_domain::enums::DateResolution::Quarter)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.project.start_date, Some(date(2026, 10, 1)));
    assert_eq!(v.project.target_date, Some(date(2026, 12, 31)));
    let bad = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                start_date: Some(Some(date(2027, 5, 1))),
                start_date_resolution: Some(None),
                target_date_resolution: Some(None),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(bad), ErrorKind::InvalidRequest);

    // Team hinzufügen, Issues des Teams im Projekt → entfernen nur mit detach, sonst invalid_request.
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                add_teams: vec!["WEB".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.teams.len(), 2);
    let issue_web = issue(&pool, org, data.web.0, 1, "backlog", None, Some(id)).await;
    let e = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                remove_teams: vec!["WEB".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["reason"], "team_has_issues");
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(id.to_string()),
                remove_teams: vec!["WEB".into()],
                detach_issues_of_removed_teams: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.teams.len(), 1);
    let (pid, hist): (Option<Uuid>, i64) = (
        sqlx::query_scalar("SELECT project_id FROM issue WHERE id = $1")
            .bind(issue_web)
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar(
            "SELECT count(*) FROM issue_history WHERE issue_id = $1 AND changes ? 'projectId'",
        )
        .bind(issue_web)
        .fetch_one(&pool)
        .await
        .unwrap(),
    );
    assert!(pid.is_none());
    assert_eq!(hist, 1);
    // Letztes Team / Lead-Team entfernen.
    assert_eq!(
        kind(
            app.save_project(
                &alice,
                SaveProjectInput {
                    id: Some(id.to_string()),
                    remove_teams: vec!["ENG".into()],
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );

    // Lead-Team-Wechsel: neue Nummer vom neuen Lead-Team, alte Kennung bleibt auflösbar.
    let old_ident = v.identifier.clone();
    let v = app
        .save_project(
            &bob,
            SaveProjectInput {
                id: Some(id.to_string()),
                add_teams: vec!["DES".into()],
                lead_team: Some("DES".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(v.identifier.starts_with("P-DES-"));
    assert_eq!(v.previous_identifiers, [old_ident.clone()]);
    let again = app
        .get_project(&alice, &old_ident, ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(again.project.id.0, id);

    // Labels: Gruppenregel und Verfügbarkeit.
    let g = Uuid::now_v7();
    sqlx::query("INSERT INTO project_label (id, organization_id, name, color, is_group) VALUES ($1,$2,'Kategorie','#000000',true)")
        .bind(g).bind(org).execute(&pool).await.unwrap();
    for (name, n) in [("Revenue", 1), ("Infra", 2)] {
        sqlx::query("INSERT INTO project_label (organization_id, name, color, parent_id) VALUES ($1,$2,'#111111',$3)")
            .bind(org).bind(name).bind(g).execute(&pool).await.unwrap();
        let _ = n;
    }
    // Alice hat Zugriff auf ENG und darf das Projekt trotz des für sie unsichtbaren DES-Teams
    // ändern (docs/04 §2.2, B5.04); DES bleibt unsichtbar.
    app.save_project(
        &alice,
        SaveProjectInput {
            id: Some(id.to_string()),
            priority: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let v = app
        .save_project(
            &bob,
            SaveProjectInput {
                id: Some(id.to_string()),
                labels: Some(vec!["Kategorie/Revenue".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.labels.len(), 1);
    let two = app
        .save_project(
            &bob,
            SaveProjectInput {
                id: Some(id.to_string()),
                labels: Some(vec!["Revenue".into(), "Infra".into()]),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(
        kind(two),
        ErrorKind::InvalidRequest,
        "höchstens ein Label je Gruppe"
    );
    assert_eq!(
        kind(
            app.save_project(
                &bob,
                SaveProjectInput {
                    id: Some(id.to_string()),
                    labels: Some(vec!["Unbekannt".into()]),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );

    // Archivierte Projekte sind schreibgeschützt.
    app.save_project(
        &bob,
        SaveProjectInput {
            id: Some(id.to_string()),
            lead: Some(Some("me".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.set_project_archived(&bob, &id.to_string(), true)
        .await
        .unwrap();
    assert_eq!(
        kind(
            app.save_project(
                &bob,
                SaveProjectInput {
                    id: Some(id.to_string()),
                    name: Some("Neu".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_reference_resolution_and_listing(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // Kennung, Slug-Id, Name und Typfehler (Issue-Kennung an Projekt-Feld).
    let seed = app
        .get_project(&alice, "P-ENG-1", ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(seed.project.id, seed_project_id());
    let by_slug = app
        .get_project(&alice, &seed.project.slug_id, ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(by_slug.project.id, seed.project.id);
    let by_full_slug = app
        .get_project(
            &alice,
            &format!("checkout-v2-{}", seed.project.slug_id),
            ViewOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_full_slug.project.id, seed.project.id);
    let by_name = app
        .get_project(&alice, "checkout V2", ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(by_name.project.id, seed.project.id);
    assert_eq!(
        kind(
            app.get_project(&alice, "ENG-123", ViewOpts::default())
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.get_project(&alice, "P-ENG-99", ViewOpts::default())
                .await
        ),
        ErrorKind::NotFound
    );

    // Filter: team, state, query, member, initiative (rekursiv); unbekannte Referenzen = not_found.
    let list = |input: ListProjectsInput| {
        let app = &app;
        let alice = &alice;
        async move { app.list_projects(alice, input).await }
    };
    let (_, r) = list(ListProjectsInput {
        team: Some("ENG".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(r.len(), 1);
    let (_, r) = list(ListProjectsInput {
        state: Some("started".into()),
        query: Some("checkout".into()),
        member: Some("me".into()),
        initiative: Some("I-1".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].initiatives[0].identifier(), "I-1");
    let (_, r) = list(ListProjectsInput {
        state: Some("completed".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(r.is_empty());
    assert_eq!(
        kind(
            list(ListProjectsInput {
                team: Some("NOPE".into()),
                ..Default::default()
            })
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            list(ListProjectsInput {
                state: Some("nope".into()),
                ..Default::default()
            })
            .await
        ),
        ErrorKind::NotFound
    );
    // includeSubTeams: WEB ist Sub-Team von ENG.
    let created = app
        .save_project(&alice, teams_input(&["WEB"]))
        .await
        .unwrap();
    let (_, r) = list(ListProjectsInput {
        team: Some("ENG".into()),
        include_sub_teams: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(r.iter().any(|p| p.project.id == created.project.id));
    let (_, r) = list(ListProjectsInput {
        team: Some("ENG".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(!r.iter().any(|p| p.project.id == created.project.id));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestones_order_progress_and_delete(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let project = "P-ENG-1";
    // Seed: Alpha (sortOrder 1). Neue Meilensteine werden ans Ende gehängt.
    let (_, beta) = app
        .save_milestone(
            &alice,
            SaveMilestoneInput {
                project: project.into(),
                name: Some("Beta".into()),
                target_date: Some(Some(date(2020, 1, 1))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(beta.milestone.sort_order, 2.0);
    assert_eq!(
        kind(
            app.save_milestone(
                &alice,
                SaveMilestoneInput {
                    project: project.into(),
                    name: Some("beta".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::Conflict,
        "Name je Projekt eindeutig"
    );
    // Issues: Alpha mit einem erledigten Issue = done; Beta (überfällig, ohne Issues).
    let alpha = lineal_testkit::seed::planning::seed_milestone_id().0;
    let i = issue(
        &pool,
        data.org.0,
        data.eng.0,
        1,
        "completed",
        None,
        Some(seed_project_id().0),
    )
    .await;
    sqlx::query("UPDATE issue SET project_milestone_id = $2 WHERE id = $1")
        .bind(i)
        .bind(alpha)
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = app.list_milestones(&alice, project).await.unwrap();
    let status: Vec<_> = list
        .iter()
        .map(|m| (m.milestone.name.as_str(), m.status.as_str(), m.progress))
        .collect();
    assert_eq!(status, [("Alpha", "done", 1.0), ("Beta", "overdue", 0.0)]);
    let (_, got) = app.get_milestone(&alice, project, "alpha").await.unwrap();
    assert_eq!((got.issue_count, got.completed_issue_count), (1, 1));
    // Update (Tri-State): Zieldatum leeren, Beschreibung setzen.
    let (_, upd) = app
        .save_milestone(
            &alice,
            SaveMilestoneInput {
                project: project.into(),
                id: Some("Beta".into()),
                description: Some("Beta-Test".into()),
                target_date: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(upd.milestone.description.as_deref(), Some("Beta-Test"));
    assert!(upd.milestone.target_date.is_none());
    // Löschen löst die Issues, History-Eintrag je Issue, IssueUpdated-Event.
    app.delete_milestone(&alice, project, "Alpha")
        .await
        .unwrap();
    let ms: Option<Uuid> =
        sqlx::query_scalar("SELECT project_milestone_id FROM issue WHERE id = $1")
            .bind(i)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(ms.is_none());
    let h: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM issue_history WHERE issue_id = $1 AND changes ? 'projectMilestoneId'",
    )
    .bind(i)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(h, 1);
    let ev: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE type = 'IssueUpdated' AND payload->'changed' ? 'projectMilestoneId'").fetch_one(&pool).await.unwrap();
    assert_eq!(ev, 1);
    assert_eq!(
        kind(app.get_milestone(&alice, project, "Alpha").await),
        ErrorKind::NotFound
    );
    // Private Projekte: Meilensteine unsichtbar.
    let bob = ctx_for(SeedUser::Bob);
    let hidden = app
        .save_project(
            &bob,
            SaveProjectInput {
                name: Some("H".into()),
                add_teams: vec!["DES".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        kind(app.list_milestones(&alice, &hidden.identifier).await),
        ErrorKind::NotFound
    );
}

// ---------------------------------------------------------------------------------------------
// Initiativen
// ---------------------------------------------------------------------------------------------

fn init(name: &str, parents: &[&str]) -> SaveInitiativeInput {
    SaveInitiativeInput {
        name: Some(name.into()),
        parent_initiatives: parents.iter().map(|p| (*p).to_owned()).collect(),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn initiative_cycle_is_invalid_request_with_reason_cycle(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // I-1 ist die Seed-Initiative; I-2 unter I-1, I-3 unter I-2.
    let i2 = app
        .save_initiative(&alice, init("Zwei", &["I-1"]))
        .await
        .unwrap();
    assert_eq!(i2.identifier, "I-2");
    let i3 = app
        .save_initiative(&alice, init("Drei", &["I-2"]))
        .await
        .unwrap();
    assert_eq!(i3.identifier, "I-3");
    assert_eq!(i3.parents[0].identifier(), "I-2");
    // I-3 als Elternteil von I-1: Zyklus.
    let e = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some("I-1".into()),
                parent_initiatives: vec!["I-3".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["reason"], "cycle");
    let path: Vec<String> = serde_json::from_value(e.details["cycle"].clone()).unwrap();
    assert_eq!(path, ["I-1", "I-2", "I-3", "I-1"]);
    // Selbstreferenz.
    let e = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some("I-2".into()),
                parent_initiatives: vec!["I-2".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.details["reason"].clone()),
        (ErrorKind::InvalidRequest, Value::from("cycle"))
    );
    // Mehrere Eltern (Diamant) sind erlaubt; doppeltes Hinzufügen ist ein No-op.
    let v = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some("I-3".into()),
                parent_initiatives: vec!["I-1".into(), "I-1".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.parents.len(), 2);
    // Tiefenlimit 5: Kette I-1 … I-5 ist ok, eine sechste Ebene scheitert mit reason max_depth.
    let i4 = app
        .save_initiative(&alice, init("Vier", &["I-3"]))
        .await
        .unwrap();
    let i5 = app
        .save_initiative(&alice, init("Fuenf", &[&i4.identifier]))
        .await
        .unwrap();
    let e = app
        .save_initiative(&alice, init("Sechs", &[&i5.identifier]))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["reason"], "max_depth");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn initiative_status_projects_visibility_and_filters(pool: PgPool) {
    let (app, _data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    // Gäste: forbidden auf Liste und Detail.
    assert_eq!(
        kind(app.get_initiative(&carol, "I-1").await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            app.list_initiatives(&carol, ListInitiativesInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    // Status-Effekte.
    let v = app.save_initiative(&alice, init("Neu", &[])).await.unwrap();
    assert_eq!(v.initiative.status.as_str(), "Planned");
    let v = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some(v.identifier),
                status: Some("started".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.initiative.status.as_str(), "Active");
    assert!(v.initiative.started_at.is_some());
    let v = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some(v.identifier),
                status: Some("Completed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(v.initiative.completed_at.is_some());
    assert_eq!(
        kind(
            app.save_initiative(
                &alice,
                SaveInitiativeInput {
                    id: Some("I-1".into()),
                    status: Some("bogus".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );

    // Lead-Team privat: für Alice nicht existent, für Bob sichtbar.
    let hidden = app
        .save_initiative(
            &bob,
            SaveInitiativeInput {
                name: Some("Design-Ziele".into()),
                lead_team: Some(Some("DES".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(hidden.visibility, "private");
    assert_eq!(
        kind(app.get_initiative(&alice, &hidden.identifier).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(app.get_initiative(&alice, "Design-Ziele").await),
        ErrorKind::NotFound
    );
    let (_, list) = app
        .list_initiatives(&alice, ListInitiativesInput::default())
        .await
        .unwrap();
    assert!(list.iter().all(|i| i.initiative.name != "Design-Ziele"));
    let (_, list) = app
        .list_initiatives(&bob, ListInitiativesInput::default())
        .await
        .unwrap();
    assert!(list.iter().any(|i| i.initiative.name == "Design-Ziele"));
    // Projekte privater Teams bleiben in sichtbaren Initiativen unsichtbar.
    let secret = app
        .save_project(
            &bob,
            SaveProjectInput {
                name: Some("Nur Design".into()),
                add_teams: vec!["DES".into()],
                add_initiatives: vec!["I-1".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let for_alice = app.get_initiative(&alice, "I-1").await.unwrap();
    assert_eq!(
        for_alice.projects.len(),
        1,
        "nur das sichtbare Seed-Projekt"
    );
    let for_bob = app.get_initiative(&bob, "I-1").await.unwrap();
    assert_eq!(for_bob.projects.len(), 2);
    // Filter.
    let (_, r) = app
        .list_initiatives(
            &alice,
            ListInitiativesInput {
                status: Some("active".into()),
                owner: Some("me".into()),
                query: Some("platform".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].initiative.id, seed_initiative_id());
    assert_eq!(
        kind(
            app.list_initiatives(
                &alice,
                ListInitiativesInput {
                    status: Some("x".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // Zuordnung per Projekt: remove/set.
    let v = app
        .save_project(
            &bob,
            SaveProjectInput {
                id: Some(secret.identifier.clone()),
                remove_initiatives: vec!["I-1".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(v.initiatives.is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM initiative_to_project")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    // Health-Rollup: Projekt `started` ohne Update zählt als „none“.
    let v = app.get_initiative(&alice, "I-1").await.unwrap();
    assert_eq!(v.health_rollup.none, 1);
}

// ---------------------------------------------------------------------------------------------
// Status-Updates
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_status_updates_health_diff_and_rules(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice); // Lead des Seed-Projekts
    let bob = ctx_for(SeedUser::Bob);
    // Erstes Update nur durch den Lead.
    assert_eq!(
        kind(
            app.post_project_update(&bob, "P-ENG-1", "Hallo", None, false)
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            app.post_project_update(&alice, "P-ENG-1", "   ", None, false)
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let first = app
        .post_project_update(&alice, "P-ENG-1", "Start.", None, false)
        .await
        .unwrap();
    assert_eq!(
        first.update.health,
        Health::OnTrack,
        "Default ohne Vorgänger"
    );
    assert!(first.update.diff.is_none());
    let p = app
        .get_project(&alice, "P-ENG-1", ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(p.project.health, Some(Health::OnTrack));
    assert_eq!(p.project.last_update_id, Some(first.update.id));
    // Ab dem zweiten Update darf jeder mit Zugriff; Health default = vorheriges; Diff bei Änderung.
    let second = app
        .post_project_update(&bob, "P-ENG-1", "Risiko.", Some(Health::AtRisk), false)
        .await
        .unwrap();
    let third = app
        .post_project_update(&bob, "P-ENG-1", "Weiter.", None, false)
        .await
        .unwrap();
    assert_eq!(third.update.health, Health::AtRisk);
    assert_eq!(second.user.as_ref().unwrap().display_name, "bob");
    // Statuswechsel + Zieldatum erscheinen im Diff des nächsten Updates.
    app.save_project(
        &alice,
        SaveProjectInput {
            id: Some("P-ENG-1".into()),
            state: Some("paused".into()),
            target_date: Some(Some(date(2027, 1, 31))),
            target_date_resolution: Some(Some(lineal_domain::enums::DateResolution::Month)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let fourth = app
        .post_project_update(&alice, "P-ENG-1", "Pause.", Some(Health::OffTrack), false)
        .await
        .unwrap();
    let md = fourth.update.diff_markdown.clone().unwrap();
    assert!(
        md.contains("Status changed from **In Progress** to **Paused**"),
        "{md}"
    );
    assert!(
        md.contains("Target date moved from **none** to **Jan 2027**"),
        "{md}"
    );
    // Bearbeiten nur durch den Autor; setzt editedAt.
    assert_eq!(
        kind(
            app.edit_project_update(&bob, &fourth.update.id.to_string(), Some("x"), None)
                .await
        ),
        ErrorKind::Forbidden
    );
    let edited = app
        .edit_project_update(
            &alice,
            &fourth.update.id.to_string(),
            Some("Pause (korrigiert)."),
            None,
        )
        .await
        .unwrap();
    assert!(edited.update.edited_at.is_some());
    assert_eq!(
        edited.update.diff_markdown, fourth.update.diff_markdown,
        "Diff wird nicht neu berechnet"
    );
    // Löschen = Archivieren; Health des Projekts fällt auf das nächstjüngere zurück.
    let (id, _) = app
        .delete_project_update(&alice, &fourth.update.id.to_string())
        .await
        .unwrap();
    assert_eq!(id, fourth.update.id);
    let p = app
        .get_project(&alice, "P-ENG-1", ViewOpts::default())
        .await
        .unwrap();
    assert_eq!(p.project.health, Some(Health::AtRisk));
    assert_eq!(p.project.last_update_id, Some(third.update.id));
    let (page, list) = app
        .list_project_updates(
            &alice,
            lineal_app::usecases::project::update::ListProjectUpdatesInput {
                project: Some("P-ENG-1".into()),
                page: PageRequest::default(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!page.has_next_page());
    assert_eq!(list.len(), 3, "archivierte Updates nur mit includeArchived");
    // Unsichtbare Projekte: not_found.
    let hidden = app
        .save_project(
            &bob,
            SaveProjectInput {
                name: Some("H".into()),
                add_teams: vec!["DES".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        kind(
            app.post_project_update(&alice, &hidden.identifier, "x", None, false)
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn initiative_status_updates(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice); // Owner von I-1
    let bob = ctx_for(SeedUser::Bob);
    assert_eq!(
        kind(
            app.post_initiative_update(&bob, "I-1", "x", None, false)
                .await
        ),
        ErrorKind::Forbidden
    );
    let u = app
        .post_initiative_update(&alice, "I-1", "Läuft.", Some(Health::AtRisk), false)
        .await
        .unwrap();
    assert_eq!(u.update.health, Health::AtRisk);
    let i = app.get_initiative(&alice, "I-1").await.unwrap();
    assert_eq!(i.initiative.health, Some(Health::AtRisk));
    let u2 = app
        .post_initiative_update(&bob, "I-1", "Update.", None, false)
        .await
        .unwrap();
    assert_eq!(u2.update.health, Health::AtRisk);
    app.delete_initiative_update(&bob, &u2.update.id.to_string())
        .await
        .unwrap();
    let carol = ctx_for(SeedUser::Carol);
    assert_eq!(
        kind(
            app.post_initiative_update(&carol, "I-1", "x", None, false)
                .await
        ),
        ErrorKind::Forbidden
    );
}

// ---------------------------------------------------------------------------------------------
// Cycles
// ---------------------------------------------------------------------------------------------

fn settings_on(first: NaiveDate) -> CycleSettingsInput {
    CycleSettingsInput {
        cycles_enabled: Some(true),
        duration: Some(2),
        cooldown: Some(0),
        start_day: Some(1),
        upcoming_count: Some(2),
        first_start_date: Some(first),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycle_boundaries_stay_on_local_midnight_across_dst(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let bob = ctx_for(SeedUser::Bob);
    let alice = ctx_for(SeedUser::Alice);
    // DES (Europe/Berlin): erster Cycle Montag 2026-10-05, Umstellung auf Winterzeit am 2026-10-25.
    app.configure_team_cycles(&bob, "DES", settings_on(date(2026, 10, 5)))
        .await
        .unwrap();
    let rows: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT number, to_char(starts_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), \
         to_char(ends_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') FROM cycle WHERE team_id = $1 ORDER BY number",
    )
    .bind(data.des.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            (
                1,
                "2026-10-04T22:00:00.000Z".to_string(),
                "2026-10-18T22:00:00.000Z".to_string()
            ),
            (
                2,
                "2026-10-18T22:00:00.000Z".to_string(),
                "2026-11-01T23:00:00.000Z".to_string()
            ),
            (
                3,
                "2026-11-01T23:00:00.000Z".to_string(),
                "2026-11-15T23:00:00.000Z".to_string()
            ),
        ]
    );
    // Der erste Cycle läuft (Uhr 2026-10-10), die beiden folgenden sind künftig; Cooldown 0 → Ende = Start.
    let (_, cycles) = app.list_cycles(&bob, "DES", None).await.unwrap();
    assert_eq!(cycles.len(), 3);
    assert!(cycles[0].is_active && cycles[1].is_next && cycles[2].is_future);
    let (_, current) = app
        .list_cycles(&bob, "des", Some(CycleKind::Current))
        .await
        .unwrap();
    assert_eq!(current[0].cycle.number, 1);
    let (_, next) = app
        .list_cycles(&bob, "DES", Some(CycleKind::Next))
        .await
        .unwrap();
    assert_eq!(next[0].cycle.number, 2);
    // Privates Team: für Alice nicht existent.
    assert_eq!(
        kind(app.list_cycles(&alice, "DES", None).await),
        ErrorKind::NotFound
    );
    // Team ohne aktivierte Cycles: leere Liste, kein Fehler.
    let (_, none) = app.list_cycles(&alice, "WEB", None).await.unwrap();
    assert!(none.is_empty());
    // Zeitplan ändern: Cooldown 1 Woche → künftige Cycles werden neu berechnet (gleiche ids und Nummern).
    let before: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM cycle WHERE team_id = $1 ORDER BY number")
            .bind(data.des.0)
            .fetch_all(&pool)
            .await
            .unwrap();
    app.configure_team_cycles(
        &bob,
        "DES",
        CycleSettingsInput {
            cooldown: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let after: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, to_char(starts_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI') FROM cycle WHERE team_id = $1 ORDER BY number",
    )
    .bind(data.des.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(after.iter().map(|a| a.0).collect::<Vec<_>>(), before);
    // Cycle 1 unverändert, Cycle 2 beginnt nach 1 Woche Cooldown: Mo 2026-10-26 00:00 lokal = 23:00Z am 25.10.
    assert_eq!(after[1].1, "2026-10-25T23:00");
    // Ungültige Werte: invalid_request mit details.field.
    let e = app
        .configure_team_cycles(
            &bob,
            "DES",
            CycleSettingsInput {
                duration: Some(9),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.details["field"].clone()),
        (ErrorKind::InvalidRequest, Value::from("cycleDuration"))
    );
    // lockToActive braucht existingActiveIssues.
    let e = app
        .configure_team_cycles(
            &bob,
            "DES",
            CycleSettingsInput {
                lock_to_active: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.details["field"], "existingActiveIssues");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sub_team_inherits_the_parent_schedule(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let bob = ctx_for(SeedUser::Bob); // Owner von WEB
    // ENG hat (Seed) Cycles; WEB schaltet Cycles ein und erbt den Zeitplan.
    app.configure_team_cycles(
        &bob,
        "WEB",
        CycleSettingsInput {
            cycles_enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let rows: Vec<(i32, Uuid, bool)> = sqlx::query_as(
        "SELECT w.number, e.id, (w.starts_at = e.starts_at AND w.ends_at = e.ends_at) FROM cycle w \
         JOIN cycle e ON e.id = w.inherited_from_id WHERE w.team_id = $1 ORDER BY w.number",
    )
    .bind(data.web.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3, "aktiver und zwei künftige ENG-Cycles");
    assert!(rows.iter().all(|r| r.2), "identische Grenzen");
    assert_eq!(rows[0].1, seed_cycle_id(2).0);
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        [1, 2, 3],
        "eigener Zähler des Sub-Teams"
    );
    // Der Zeitplan des Parents ist für das Sub-Team nicht änderbar.
    let e = app
        .configure_team_cycles(
            &bob,
            "WEB",
            CycleSettingsInput {
                duration: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.details["reason"].clone()),
        (ErrorKind::InvalidRequest, Value::from("inherited_schedule"))
    );
    // Datumsänderung am Sub-Team-Cycle ist nicht erlaubt.
    let (_, web) = app
        .list_cycles(&bob, "WEB", Some(CycleKind::Next))
        .await
        .unwrap();
    let e = app
        .update_cycle(
            &bob,
            lineal_app::usecases::cycle::UpdateCycleInput {
                id: web[0].cycle.id.to_string(),
                ends_at: Some(utc(2026, 12, 1, 0, 0)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    // Parent verschiebt das Ende des aktiven Cycles → das Sub-Team zieht nach.
    let alice = ctx_for(SeedUser::Alice);
    let (_, eng_next) = app
        .list_cycles(&alice, "ENG", Some(CycleKind::Next))
        .await
        .unwrap();
    let _ = eng_next;
    let (_, eng_cur) = app
        .list_cycles(&alice, "ENG", Some(CycleKind::Current))
        .await
        .unwrap();
    let new_end = utc(2026, 10, 19, 22, 0); // einen Tag später
    app.update_cycle(
        &alice,
        lineal_app::usecases::cycle::UpdateCycleInput {
            id: eng_cur[0].cycle.id.to_string(),
            ends_at: Some(new_end),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (e_end, w_end): (DateTime<Utc>, DateTime<Utc>) = (
        sqlx::query_scalar("SELECT ends_at FROM cycle WHERE id = $1")
            .bind(seed_cycle_id(2).0)
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT ends_at FROM cycle WHERE team_id = $1 AND number = 1")
            .bind(data.web.0)
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!((e_end, w_end), (new_end, new_end));
    // Nachfolger beginnt nun später (Ende unverändert).
    let next_start: DateTime<Utc> = sqlx::query_scalar("SELECT starts_at FROM cycle WHERE id = $1")
        .bind(seed_cycle_id(3).0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(next_start, new_end);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycle_transition_rolls_over_unfinished_issues(pool: PgPool) {
    let (_app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let org = data.org.0;
    // Eigenes Team OPS (Berlin). Am 2026-09-25 beginnt der erste Cycle (Mo 2026-09-21 → 2026-10-05).
    let early = app_at(&pool, utc(2026, 9, 25, 9, 0));
    let ops = new_team(&early, &alice, "Ops", "OPS", "Europe/Berlin").await;
    early
        .configure_team_cycles(&alice, "OPS", settings_on(date(2026, 9, 21)))
        .await
        .unwrap();
    let c1: (Uuid, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT id, completed_at FROM cycle WHERE team_id = $1 AND number = 1")
            .bind(ops)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(c1.1.is_none(), "Cycle 1 läuft am 25.09.");
    let started = issue(&pool, org, ops, 1, "started", Some(c1.0), None).await;
    let todo = issue(&pool, org, ops, 2, "unstarted", Some(c1.0), None).await;
    let backlog = issue(&pool, org, ops, 3, "backlog", Some(c1.0), None).await;
    let done = issue(&pool, org, ops, 4, "completed", Some(c1.0), None).await;
    let canceled = issue(&pool, org, ops, 5, "canceled", Some(c1.0), None).await;
    // Später (2026-10-10): Cycle 1 ist abgelaufen, Cycle 2 (bis 2026-10-19) läuft.
    let late = app_at(&pool, utc(2026, 10, 10, 7, 15));
    let stats = late.run_cycle_maintenance(late.clock.now()).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    let c2: Uuid = sqlx::query_scalar("SELECT id FROM cycle WHERE team_id = $1 AND number = 2")
        .bind(ops)
        .fetch_one(&pool)
        .await
        .unwrap();
    let cyc = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<Uuid>>("SELECT cycle_id FROM issue WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(cyc(started).await, Some(c2), "started rollt über");
    assert_eq!(cyc(todo).await, Some(c2), "unstarted rollt über");
    assert_eq!(cyc(backlog).await, None, "backlog verliert den Cycle");
    assert_eq!(
        cyc(done).await,
        Some(c1.0),
        "erledigte bleiben im alten Cycle"
    );
    assert_eq!(cyc(canceled).await, Some(c1.0));
    // Abschluss von Cycle 1: eingefroren, Endzeit = geplantes Ende, offene Issues festgehalten.
    let (completed, ends, uncompleted, progress): (Option<DateTime<Utc>>, DateTime<Utc>, Option<Vec<Uuid>>, Option<f64>) =
        sqlx::query_as("SELECT completed_at, ends_at, uncompleted_issue_ids_upon_close, progress FROM cycle WHERE id = $1").bind(c1.0).fetch_one(&pool).await.unwrap();
    assert_eq!(completed, Some(ends));
    assert_eq!(ends, utc(2026, 10, 4, 22, 0));
    let mut u = uncompleted.unwrap();
    u.sort();
    let mut expected = vec![started, todo, backlog];
    expected.sort();
    assert_eq!(u, expected);
    assert!(progress.is_some());
    // Cycle 2 ist gestartet (Tag-0-Snapshot), Cycle 3 und 4 existieren künftig.
    let (sp, hist): (Option<DateTime<Utc>>, Vec<f64>) =
        sqlx::query_as("SELECT started_processed_at, scope_history FROM cycle WHERE id = $1")
            .bind(c2)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sp.is_some());
    assert_eq!(
        hist.first().copied(),
        Some(2.0),
        "Tag-0-Scope: zwei übernommene Issues"
    );
    let future: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cycle WHERE team_id = $1 AND starts_at > $2")
            .bind(ops)
            .bind(utc(2026, 10, 10, 7, 15))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(future, 2);
    // History (Quelle system, changes.cycleId) und Events (IssueUpdated changed=[cycleId], Origin system).
    let h: Vec<(String, Value)> =
        sqlx::query_as("SELECT source, changes FROM issue_history WHERE issue_id = $1")
            .bind(started)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].0, "system");
    assert_eq!(h[0].1["cycleId"]["from"], c1.0.to_string());
    assert_eq!(h[0].1["cycleId"]["to"], c2.to_string());
    let ev: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE type = 'IssueUpdated' AND origin = 'system' AND payload->'changed' = '[\"cycleId\"]'::jsonb").fetch_one(&pool).await.unwrap();
    assert_eq!(ev, 3);
    let cu: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE type = 'CycleUpdated' AND payload->>'cycle_id' = $1").bind(c1.0.to_string()).fetch_one(&pool).await.unwrap();
    assert!(cu >= 1);
    // Idempotenz: ein zweiter Lauf ändert nichts mehr.
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM issue_history")
        .fetch_one(&pool)
        .await
        .unwrap();
    late.run_cycle_maintenance(late.clock.now()).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM issue_history")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(cyc(started).await, Some(c2));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycle_cooldown_keeps_issues_until_the_next_cycle_starts(pool: PgPool) {
    let (_app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let org = data.org.0;
    // 1 Woche Cooldown: Cycle 1 = Mo 2026-09-21 bis 2026-10-05, Cycle 2 ab Mo 2026-10-12.
    let early = app_at(&pool, utc(2026, 9, 25, 9, 0));
    let ops = new_team(&early, &alice, "Ops", "OPS", "Europe/Berlin").await;
    early
        .configure_team_cycles(
            &alice,
            "OPS",
            CycleSettingsInput {
                cooldown: Some(1),
                ..settings_on(date(2026, 9, 21))
            },
        )
        .await
        .unwrap();
    let c1: Uuid = sqlx::query_scalar("SELECT id FROM cycle WHERE team_id = $1 AND number = 1")
        .bind(ops)
        .fetch_one(&pool)
        .await
        .unwrap();
    let open = issue(&pool, org, ops, 1, "started", Some(c1), None).await;
    let moved_to_backlog = issue(&pool, org, ops, 2, "started", Some(c1), None).await;
    // Cooldown (2026-10-07): Cycle 1 ist abgeschlossen, die Issues bleiben bis zum Start von Cycle 2 dran.
    let cooldown = app_at(&pool, utc(2026, 10, 7, 9, 0));
    cooldown
        .run_cycle_maintenance(cooldown.clock.now())
        .await
        .unwrap();
    let cyc = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<Uuid>>("SELECT cycle_id FROM issue WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(cyc(open).await, Some(c1));
    assert!(
        sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT completed_at FROM cycle WHERE id = $1"
        )
        .bind(c1)
        .fetch_one(&pool)
        .await
        .unwrap()
        .is_some()
    );
    let (_, cur) = cooldown
        .list_cycles(&alice, "OPS", Some(CycleKind::Current))
        .await
        .unwrap();
    assert!(cur.is_empty(), "im Cooldown gibt es keinen aktiven Cycle");
    // Während des Cooldowns wandert ein Issue in den Backlog: es wird nicht übernommen.
    let backlog_state = state_id(&pool, ops, "backlog").await;
    sqlx::query("UPDATE issue SET state_id = $2 WHERE id = $1")
        .bind(moved_to_backlog)
        .bind(backlog_state)
        .execute(&pool)
        .await
        .unwrap();
    let after = app_at(&pool, utc(2026, 10, 13, 9, 0));
    after
        .run_cycle_maintenance(after.clock.now())
        .await
        .unwrap();
    let c2: Uuid = sqlx::query_scalar("SELECT id FROM cycle WHERE team_id = $1 AND number = 2")
        .bind(ops)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cyc(open).await, Some(c2));
    assert_eq!(cyc(moved_to_backlog).await, None);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn deactivating_cycles_closes_the_active_one_and_removes_upcoming(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let org = data.org.0;
    let ops = new_team(&app, &alice, "Ops", "OPS", "Europe/Berlin").await;
    app.configure_team_cycles(&alice, "OPS", settings_on(date(2026, 10, 5)))
        .await
        .unwrap();
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM cycle WHERE team_id = $1 ORDER BY number")
            .bind(ops)
            .fetch_all(&pool)
            .await
            .unwrap();
    let in_active = issue(&pool, org, ops, 1, "started", Some(ids[0]), None).await;
    let in_future = issue(&pool, org, ops, 2, "unstarted", Some(ids[1]), None).await;
    app.configure_team_cycles(
        &alice,
        "OPS",
        CycleSettingsInput {
            cycles_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cycle WHERE team_id = $1")
        .bind(ops)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 1, "nur der abgeschlossene Cycle bleibt");
    let completed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT completed_at FROM cycle WHERE id = $1")
            .bind(ids[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(completed.is_some());
    for i in [in_active, in_future] {
        let c: Option<Uuid> = sqlx::query_scalar("SELECT cycle_id FROM issue WHERE id = $1")
            .bind(i)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(c.is_none());
    }
    // Reaktivieren setzt die Nummerierung fort.
    app.configure_team_cycles(
        &alice,
        "OPS",
        settings_on(date(2026, 10, 5) + chrono::Duration::days(14)),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let max: i32 = sqlx::query_scalar("SELECT max(number) FROM cycle WHERE team_id = $1")
        .bind(ops)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(max >= 4);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycle_shift_start_today_and_archive(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // Seed-ENG: #2 aktiv, #3/#4 künftig.
    let c3 = seed_cycle_id(3).to_string();
    let views = app.shift_cycles(&alice, &c3, 1).await.unwrap();
    assert_eq!(views.len(), 2, "der Cycle und alle späteren");
    let start: DateTime<Utc> = sqlx::query_scalar("SELECT starts_at FROM cycle WHERE id = $1")
        .bind(seed_cycle_id(3).0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        start,
        utc(2026, 10, 19, 22, 0),
        "lokal Di 2026-10-20 00:00 (Sommerzeit)"
    );
    // Nicht in die Vergangenheit / nicht über den Vorgänger.
    assert_eq!(
        kind(app.shift_cycles(&alice, &c3, -30).await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.shift_cycles(&alice, &seed_cycle_id(2).to_string(), 1)
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // „Start cycle today“: nur der nächste künftige; schließt den laufenden Cycle und rollt über.
    let issue_started = issue(
        &pool,
        data.org.0,
        data.eng.0,
        1,
        "started",
        Some(seed_cycle_id(2).0),
        None,
    )
    .await;
    assert_eq!(
        kind(
            app.start_cycle_today(&alice, &seed_cycle_id(4).to_string())
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let v = app.start_cycle_today(&alice, &c3).await.unwrap();
    assert!(v.is_active);
    assert_eq!(
        v.cycle.starts_at,
        utc(2026, 10, 9, 22, 0),
        "heute 00:00 Berlin (2026-10-10, UTC+2)"
    );
    let (completed, ends): (Option<DateTime<Utc>>, DateTime<Utc>) =
        sqlx::query_as("SELECT completed_at, ends_at FROM cycle WHERE id = $1")
            .bind(seed_cycle_id(2).0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (completed, ends),
        (Some(utc(2026, 10, 9, 22, 0)), utc(2026, 10, 9, 22, 0))
    );
    let now_cycle: Option<Uuid> = sqlx::query_scalar("SELECT cycle_id FROM issue WHERE id = $1")
        .bind(issue_started)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(now_cycle, Some(seed_cycle_id(3).0));
    // Archivieren: Issues verlieren den Cycle, der aktive Cycle ist tabu.
    assert_eq!(
        kind(app.archive_cycle(&alice, &c3).await),
        ErrorKind::InvalidRequest
    );
    app.archive_cycle(&alice, &seed_cycle_id(4).to_string())
        .await
        .unwrap();
    let (_, listed) = app.list_cycles(&alice, "ENG", None).await.unwrap();
    assert!(listed.iter().all(|c| c.cycle.id != seed_cycle_id(4)));
}

// ---------------------------------------------------------------------------------------------
// Regressionstests Security-Review: Rechte über alle Teams, Bereinigung gespeicherter Inhalte
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn changes_need_rights_on_all_current_and_added_teams(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let shared = app
        .save_project(
            &bob,
            SaveProjectInput {
                lead_team: Some("ENG".into()),
                ..teams_input(&["ENG", "DES"])
            },
        )
        .await
        .unwrap();
    let edit = |ctx: &RequestCtx, input: SaveProjectInput| {
        let app = &app;
        let ctx = ctx.clone();
        async move { app.save_project(&ctx, input).await }
    };
    // Alice sieht die Hülle und darf mit Zugriff auf ENG ändern; DES bleibt für sie unsichtbar
    // (docs/04 §2.2: Zugriff auf mindestens ein Projekt-Team genügt).
    assert!(
        app.get_project(&alice, &shared.identifier, ViewOpts::default())
            .await
            .is_ok()
    );
    let id = Some(shared.identifier.clone());
    for input in [
        SaveProjectInput {
            id: id.clone(),
            name: Some("Neu".into()),
            ..Default::default()
        },
        SaveProjectInput {
            id: id.clone(),
            state: Some("started".into()),
            ..Default::default()
        },
        // setTeams ohne das unsichtbare DES behält DES bei.
        SaveProjectInput {
            id: id.clone(),
            set_teams: Some(vec!["ENG".into()]),
            ..Default::default()
        },
    ] {
        edit(&alice, input).await.unwrap();
    }
    let teams: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM project_team WHERE project_id = $1")
            .bind(shared.project.id.0)
            .fetch_all(&_pool)
            .await
            .unwrap();
    assert_eq!(teams.len(), 2, "DES bleibt am Projekt");
    // DES kann Alice weder entfernen noch referenzieren.
    assert_eq!(
        kind(
            edit(
                &alice,
                SaveProjectInput {
                    id: id.clone(),
                    remove_teams: vec!["DES".into()],
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    app.post_project_update(&alice, &shared.identifier, "x", None, false)
        .await
        .unwrap();
    app.save_milestone(
        &alice,
        SaveMilestoneInput {
            project: shared.identifier.clone(),
            name: Some("M".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Ein Team-Mitglied ohne Zugriff auf das private Team kann es nicht hinzufügen.
    let eng_only = app
        .save_project(&alice, teams_input(&["ENG"]))
        .await
        .unwrap();
    assert_eq!(
        kind(
            edit(
                &alice,
                SaveProjectInput {
                    id: Some(eng_only.identifier.clone()),
                    add_teams: vec!["DES".into()],
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            edit(
                &alice,
                SaveProjectInput {
                    id: Some(eng_only.identifier.clone()),
                    set_teams: Some(vec!["DES".into()]),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // Lead ohne Zugriff auf ein Projekt-Team ist nicht zulässig (Gast Carol: nur WEB).
    let e = edit(
        &alice,
        SaveProjectInput {
            id: Some(eng_only.identifier.clone()),
            lead: Some(Some("carol".into())),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(kind(e), ErrorKind::InvalidRequest);
    // Mit Rechten für alle Teams geht es (Bob).
    assert!(
        edit(
            &bob,
            SaveProjectInput {
                id: id.clone(),
                name: Some("Neu".into()),
                ..Default::default()
            }
        )
        .await
        .is_ok()
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn stored_content_is_sanitized_and_validated(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let v = app
        .save_project(
            &alice,
            SaveProjectInput {
                name: Some("<script>alert(1)</script>Safe".into()),
                summary: Some("<img src=x onerror=alert(1)>kurz".into()),
                description: Some(
                    "Hallo <script>alert(1)</script> [klick](javascript:alert(1)) **fett**".into(),
                ),
                icon: Some("Rocket".into()),
                color: Some("#AABBCC".into()),
                links: vec![("https://example.com/a".into(), "<b>Doku</b>".into())],
                ..teams_input(&["ENG"])
            },
        )
        .await
        .unwrap();
    assert!(!v.project.name.contains('<') && v.project.name.contains("Safe"));
    assert!(!v.project.description.clone().unwrap().contains('<'));
    let content = v.project.content.clone().unwrap();
    assert!(
        !content.contains("<script") && !content.contains("javascript:"),
        "{content}"
    );
    assert!(content.contains("**fett**"));
    // Ungültige Farbe, Icon und Link-Schema: invalid_request.
    let id = Some(v.identifier.clone());
    for input in [
        SaveProjectInput {
            id: id.clone(),
            color: Some("red".into()),
            ..Default::default()
        },
        SaveProjectInput {
            id: id.clone(),
            color: Some("#12345".into()),
            ..Default::default()
        },
        SaveProjectInput {
            id: id.clone(),
            icon: Some("<svg onload=x>".into()),
            ..Default::default()
        },
        SaveProjectInput {
            id: id.clone(),
            links: vec![("javascript:alert(1)".into(), "x".into())],
            ..Default::default()
        },
        SaveProjectInput {
            id: id.clone(),
            links: vec![("data:text/html,x".into(), "x".into())],
            ..Default::default()
        },
    ] {
        assert_eq!(
            kind(app.save_project(&alice, input).await),
            ErrorKind::InvalidRequest
        );
    }
    // Status-Update-Body, Meilenstein-Beschreibung und Initiative werden bereinigt.
    let u = app
        .post_project_update(
            &alice,
            "P-ENG-1",
            "ok <script>x</script> [a](javascript:x)",
            None,
            false,
        )
        .await
        .unwrap();
    assert!(!u.update.body.contains("<script") && !u.update.body.contains("javascript:"));
    let (_, m) = app
        .save_milestone(
            &alice,
            SaveMilestoneInput {
                project: "P-ENG-1".into(),
                name: Some("<i>M</i>".into()),
                description: Some("<script>x</script>text".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!m.milestone.name.contains('<'));
    assert!(!m.milestone.description.clone().unwrap().contains("<script"));
    let i = app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                name: Some("<b>Ini</b>".into()),
                summary: Some("<script>x</script>s".into()),
                description: Some("<script>x</script>d".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!i.initiative.name.contains('<'));
    assert!(
        !i.initiative
            .content
            .clone()
            .unwrap_or_default()
            .contains("<script")
    );
    assert_eq!(
        kind(
            app.save_initiative(
                &alice,
                SaveInitiativeInput {
                    name: Some("X".into()),
                    color: Some("javascript:1".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.save_initiative(
                &alice,
                SaveInitiativeInput {
                    name: Some("X".into()),
                    icon: Some("a b<".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
}
