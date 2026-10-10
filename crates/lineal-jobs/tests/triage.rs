#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP12: Triage-Aktionen, Parent/Child-Automation, Auto-Cycle, Auto-Close, Auto-Archive, Purge,
//! SLA und Rotation gegen echte Datenbank (Seed `acme`).

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::ports::repos::triage::TriageAction;
use lineal_app::usecases::comment::{CreateCommentInput, ParentSpec};
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::team::UpdateTeamInput;
use lineal_app::usecases::triage::{
    ScheduleEntryInput, SetTriageResponsibilityInput, TriageAcceptInput, TriageDeclineInput,
    TriageDuplicateInput, TriageMoveInput, TriageSnoozeInput,
};
use lineal_app::{App, AppError, ErrorKind, RequestCtx, Scopes};
use lineal_db::PgStore;
use lineal_domain::enums::StateType;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::auto_archive::AutoArchiveJob;
use lineal_jobs::periodic::auto_close::AutoCloseJob;
use lineal_jobs::periodic::purge::PurgeJob;
use lineal_jobs::periodic::sla::SlaJob;
use lineal_jobs::periodic::triage_rotation::TriageRotationJob;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Fx {
    app: Arc<App>,
    data: SeedData,
    pool: PgPool,
}

/// Die feste Test-Uhr der Testkit (`2026-10-10T07:15:00Z`).
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Fx {
        app: test_app(Arc::new(store)),
        data,
        pool,
    }
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}
fn carol() -> RequestCtx {
    ctx_for(SeedUser::Carol)
}

fn kind<T>(r: Result<T, AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn create(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

fn update(id: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        id: id.into(),
        ..Default::default()
    }
}

async fn enable_triage(fx: &Fx, ctx: &RequestCtx, team: &str) {
    fx.app
        .update_team(
            ctx,
            team,
            UpdateTeamInput {
                triage_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn triage_issue(fx: &Fx, title: &str) -> String {
    let i = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                state: Some("triage".into()),
                ..create("ENG", title)
            },
        )
        .await
        .unwrap();
    assert_eq!(i.issue.state_type, StateType::Triage);
    i.issue.identifier
}

async fn state_of(fx: &Fx, ident: &str) -> StateType {
    fx.app
        .get_issue(&alice(), ident)
        .await
        .unwrap()
        .view
        .issue
        .state_type
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

// --- Triage-Aktionen ----------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn accept_moves_to_default_state_and_comments(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let id = triage_issue(&fx, "Eingang").await;
    let v = fx
        .app
        .triage_accept(
            &bob(),
            TriageAcceptInput {
                issue: id.clone(),
                comment: Some("Passt, wir nehmen es.".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // defaultIssueState des Seed-Teams = Backlog
    assert_eq!(v.issue.state_id, seed_workflow_state_ids(1)[0]);
    assert_eq!(v.issue.state_type, StateType::Backlog);
    assert!(v.issue.triaged_at.is_some());
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM comment WHERE body LIKE '%wir nehmen%'"
        )
        .await,
        1
    );
    // IssueTriaged wurde geschrieben
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM outbox_event WHERE payload->>'type' = 'IssueTriaged' AND payload->>'action' = 'accept'"
        )
        .await,
        1
    );
    // zweites Accept: nicht mehr in Triage
    assert_eq!(
        kind(
            fx.app
                .triage_accept(
                    &bob(),
                    TriageAcceptInput {
                        issue: id,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn accept_with_chosen_state_and_priority_requirement(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                require_priority_to_leave_triage: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = triage_issue(&fx, "Braucht Priorität").await;
    let err = fx
        .app
        .triage_accept(
            &bob(),
            TriageAcceptInput {
                issue: id.clone(),
                ..Default::default()
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    // Zielstatus muss backlog/unstarted/started sein
    assert_eq!(
        kind(
            fx.app
                .triage_accept(
                    &bob(),
                    TriageAcceptInput {
                        issue: id.clone(),
                        state: Some("Done".into()),
                        priority: Some(2),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let v = fx
        .app
        .triage_accept(
            &bob(),
            TriageAcceptInput {
                issue: id,
                state: Some("In Progress".into()),
                priority: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.state_type, StateType::Started);
    assert_eq!(v.issue.priority.value(), 2);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn decline_cancels_with_comment(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let id = triage_issue(&fx, "Unsinn").await;
    let v = fx
        .app
        .triage_decline(
            &bob(),
            TriageDeclineInput {
                issue: id,
                comment: Some("Das bauen wir nicht.".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.state_type, StateType::Canceled);
    assert_eq!(v.issue.state_id, seed_workflow_state_ids(1)[4]);
    assert!(v.issue.canceled_at.is_some() && v.issue.triaged_at.is_some());
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM comment WHERE body LIKE '%nicht.%'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn duplicate_via_triage_sets_state_relation_and_moves_attachments(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let canonical = fx
        .app
        .create_issue(&alice(), create("ENG", "Original"))
        .await
        .unwrap();
    let dup = triage_issue(&fx, "Nochmal").await;
    let dup_id = fx
        .app
        .get_issue(&alice(), &dup)
        .await
        .unwrap()
        .view
        .issue
        .id;
    sqlx::query(
        "INSERT INTO attachment (organization_id, issue_id, url, title) VALUES ($1, $2, 'https://example.com/log', 'Log')",
    )
    .bind(fx.data.org.0)
    .bind(dup_id.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    let v = fx
        .app
        .triage_duplicate(
            &bob(),
            TriageDuplicateInput {
                issue: dup.clone(),
                duplicate_of: canonical.issue.identifier.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.state_type, StateType::Duplicate);
    let rel = count(
        &fx.pool,
        "SELECT count(*) FROM issue_relation WHERE type = 'duplicate'",
    )
    .await;
    assert_eq!(rel, 1);
    let moved: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachment WHERE issue_id = $1 AND original_issue_id = $2",
    )
    .bind(canonical.issue.id.0)
    .bind(dup_id.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(moved, 1, "Anhänge wandern zum kanonischen Issue");
    // nicht aus Triage: invalid_request
    assert_eq!(
        kind(
            fx.app
                .triage_duplicate(
                    &bob(),
                    TriageDuplicateInput {
                        issue: canonical.issue.identifier.clone(),
                        duplicate_of: dup,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn snooze_hides_from_triage_until_the_job_resurfaces_it(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let id = triage_issue(&fx, "Später").await;
    let _other = triage_issue(&fx, "Sofort").await;
    let until = t0() + Duration::days(1);
    // Vergangenheit ist ungültig
    assert_eq!(
        kind(
            fx.app
                .triage_snooze(
                    &bob(),
                    TriageSnoozeInput {
                        issue: id.clone(),
                        until: t0() - Duration::hours(1),
                        source: HistorySource::User,
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let v = fx
        .app
        .triage_snooze(
            &bob(),
            TriageSnoozeInput {
                issue: id.clone(),
                until,
                source: HistorySource::User,
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.snoozed_until_at, Some(until));
    assert_eq!(v.issue.snoozed_by_id, Some(fx.data.bob));
    let visible = fx
        .app
        .list_triage_issues(&bob(), "ENG", false, 50)
        .await
        .unwrap();
    assert_eq!(
        visible.len(),
        1,
        "gesnoozte Issues sind für alle ausgeblendet"
    );
    assert_eq!(
        fx.app
            .list_triage_issues(&alice(), "ENG", true, 50)
            .await
            .unwrap()
            .len(),
        2
    );
    // Vor dem Termin passiert nichts, danach taucht das Issue wieder auf.
    TriageRotationJob
        .run(&fx.app, t0() + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(
        fx.app
            .list_triage_issues(&bob(), "ENG", false, 50)
            .await
            .unwrap()
            .len(),
        1
    );
    TriageRotationJob
        .run(&fx.app, until + Duration::minutes(1))
        .await
        .unwrap();
    // Die App-Uhr steht fest bei t0, `list_triage_issues` zählt abgelaufene Snoozes ohnehin mit.
    let list = fx
        .app
        .list_triage_issues(&bob(), "ENG", false, 50)
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    let after = fx.app.get_issue(&bob(), &id).await.unwrap().view.issue;
    assert!(after.snoozed_until_at.is_none() && after.snoozed_by_id.is_none());
    // idempotent
    TriageRotationJob
        .run(&fx.app, until + Duration::minutes(2))
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn move_to_team_changes_identifier(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let id = triage_issue(&fx, "Falsches Team").await;
    // Bob ist Mitglied in ENG und WEB: im Zielteam gilt die Triage-Regel erneut (Mitglied → Mapping).
    let v = fx
        .app
        .triage_move(
            &bob(),
            TriageMoveInput {
                issue: id,
                team: "WEB".into(),
                source: HistorySource::User,
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.team_id, fx.data.web);
    assert!(v.issue.identifier.starts_with("WEB-"));
    assert_ne!(v.issue.state_type, StateType::Triage);
}

// --- Sicherheit -------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn triage_on_private_team_is_not_found_for_non_members_and_read_scope_is_forbidden(
    pool: PgPool,
) {
    let fx = setup(pool).await;
    // DES ist privat; nur Bob ist Mitglied.
    enable_triage(&fx, &bob(), "DES").await;
    let des = fx
        .app
        .create_issue(
            &bob(),
            CreateIssueInput {
                state: Some("triage".into()),
                ..create("DES", "Geheim")
            },
        )
        .await
        .unwrap();
    let ident = des.issue.identifier.clone();
    for ctx in [alice(), carol()] {
        assert_eq!(
            kind(
                fx.app
                    .triage_accept(
                        &ctx,
                        TriageAcceptInput {
                            issue: ident.clone(),
                            ..Default::default()
                        }
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                fx.app
                    .triage_decline(
                        &ctx,
                        TriageDeclineInput {
                            issue: des.issue.id.to_string(),
                            ..Default::default()
                        }
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                fx.app
                    .triage_snooze(
                        &ctx,
                        TriageSnoozeInput {
                            issue: ident.clone(),
                            until: t0() + Duration::days(1),
                            source: HistorySource::User,
                        }
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                fx.app
                    .triage_move(
                        &ctx,
                        TriageMoveInput {
                            issue: ident.clone(),
                            team: "ENG".into(),
                            source: HistorySource::User
                        }
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(fx.app.list_triage_issues(&ctx, "DES", false, 10).await),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                fx.app
                    .get_triage_responsibility(&ctx, "DES", None, 10)
                    .await
            ),
            ErrorKind::NotFound
        );
    }
    // Als Duplikat-Ziel eines sichtbaren Issues ist ein privates Issue ebenfalls unsichtbar.
    let eng = triage_issue_in_eng(&fx).await;
    assert_eq!(
        kind(
            fx.app
                .triage_duplicate(
                    &alice(),
                    TriageDuplicateInput {
                        issue: eng,
                        duplicate_of: ident.clone(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Lese-Token: forbidden (auch für Mitglieder), nichts wurde verändert.
    let read_only = RequestCtx {
        scopes: Scopes::read_only(),
        ..bob()
    };
    assert_eq!(
        kind(
            fx.app
                .triage_accept(
                    &read_only,
                    TriageAcceptInput {
                        issue: ident.clone(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            fx.app
                .triage_decline(
                    &read_only,
                    TriageDeclineInput {
                        issue: ident.clone(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    let still = fx.app.get_issue(&bob(), &ident).await.unwrap();
    assert_eq!(still.view.issue.state_type, StateType::Triage);
}

async fn triage_issue_in_eng(fx: &Fx) -> String {
    enable_triage(fx, &alice(), "ENG").await;
    triage_issue(fx, "Sichtbar").await
}

// --- Parent/Child-Automation, Auto-Cycle ---------------------------------------------------

async fn set_team_flags(fx: &Fx, parent: bool, child: bool) {
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                auto_close_parent_issues: Some(parent),
                auto_close_child_issues: Some(child),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn sub_issue(fx: &Fx, parent: &str, title: &str) -> String {
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                parent: Some(parent.into()),
                ..create("ENG", title)
            },
        )
        .await
        .unwrap()
        .issue
        .identifier
}

fn to_state(id: &str, state: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        state: Some(state.into()),
        ..update(id)
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn parent_becomes_done_when_all_children_are_done_and_vice_versa(pool: PgPool) {
    let fx = setup(pool).await;
    set_team_flags(&fx, true, true).await;
    let parent = fx
        .app
        .create_issue(&alice(), create("ENG", "Epic"))
        .await
        .unwrap()
        .issue
        .identifier;
    let c1 = sub_issue(&fx, &parent, "Teil 1").await;
    let c2 = sub_issue(&fx, &parent, "Teil 2").await;
    fx.app
        .update_issue(&alice(), to_state(&c1, "Done"))
        .await
        .unwrap();
    assert_eq!(
        state_of(&fx, &parent).await,
        StateType::Backlog,
        "noch ein offenes Child"
    );
    fx.app
        .update_issue(&alice(), to_state(&c2, "Done"))
        .await
        .unwrap();
    assert_eq!(state_of(&fx, &parent).await, StateType::Completed);
    // History des Parents: Quelle automation, kein Akteur
    let (src, actor): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT h.source, h.actor_id FROM issue_history h JOIN issue i ON i.id = h.issue_id \
         WHERE i.identifier = $1 AND h.changes ? 'stateId' ORDER BY h.created_at DESC LIMIT 1",
    )
    .bind(&parent)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(src, "automation");
    assert!(actor.is_none());

    // Umgekehrt: Parent abgebrochen → offene Children werden abgebrochen.
    let p2 = fx
        .app
        .create_issue(&alice(), create("ENG", "Epic 2"))
        .await
        .unwrap()
        .issue
        .identifier;
    let k1 = sub_issue(&fx, &p2, "Kind 1").await;
    let k2 = sub_issue(&fx, &p2, "Kind 2").await;
    fx.app
        .update_issue(&alice(), to_state(&k1, "Done"))
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&p2, "Canceled"))
        .await
        .unwrap();
    assert_eq!(state_of(&fx, &k2).await, StateType::Canceled);
    assert_eq!(
        state_of(&fx, &k1).await,
        StateType::Completed,
        "geschlossene Children bleiben"
    );
    let flagged: bool =
        sqlx::query_scalar("SELECT auto_closed_by_parent_closing FROM issue WHERE identifier = $1")
            .bind(&k2)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert!(flagged);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn automation_is_off_by_default_and_ignores_all_canceled_children(pool: PgPool) {
    let fx = setup(pool).await;
    let parent = fx
        .app
        .create_issue(&alice(), create("ENG", "Epic"))
        .await
        .unwrap()
        .issue
        .identifier;
    let c1 = sub_issue(&fx, &parent, "Teil").await;
    fx.app
        .update_issue(&alice(), to_state(&c1, "Done"))
        .await
        .unwrap();
    assert_eq!(
        state_of(&fx, &parent).await,
        StateType::Backlog,
        "Einstellung aus"
    );
    set_team_flags(&fx, true, false).await;
    let p2 = fx
        .app
        .create_issue(&alice(), create("ENG", "Epic 2"))
        .await
        .unwrap()
        .issue
        .identifier;
    let k = sub_issue(&fx, &p2, "Nur abgebrochen").await;
    fx.app
        .update_issue(&alice(), to_state(&k, "Canceled"))
        .await
        .unwrap();
    assert_eq!(
        state_of(&fx, &p2).await,
        StateType::Backlog,
        "alle Children canceled → Parent bleibt"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn backlog_issue_in_active_cycle_becomes_unstarted(pool: PgPool) {
    let fx = setup(pool).await;
    let i = fx
        .app
        .create_issue(&alice(), create("ENG", "Cycle-Kandidat"))
        .await
        .unwrap();
    assert_eq!(i.issue.state_type, StateType::Backlog);
    // Seed-Cycle #2 läuft zur Test-Uhr.
    let v = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                cycle: Some(Some("2".into())),
                ..update(&i.issue.identifier)
            },
        )
        .await
        .unwrap();
    assert_eq!(v.issue.state_type, StateType::Unstarted);
    assert!(v.issue.cycle_id.is_some());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn state_change_auto_assigns_the_active_cycle(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                cycle_issue_auto_assign_started: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let i = fx
        .app
        .create_issue(&alice(), create("ENG", "Wird gestartet"))
        .await
        .unwrap();
    assert!(i.issue.cycle_id.is_none());
    let v = fx
        .app
        .update_issue(&alice(), to_state(&i.issue.identifier, "In Progress"))
        .await
        .unwrap();
    assert!(
        v.issue.cycle_id.is_some(),
        "gestartete Issues kommen in den aktiven Cycle"
    );
    assert!(v.issue.added_to_cycle_at.is_some());
}

// --- Auto-Close ---------------------------------------------------------------------------

async fn age(fx: &Fx, ident: &str, at: DateTime<Utc>) {
    sqlx::query("UPDATE issue SET updated_at = $2 WHERE identifier = $1")
        .bind(ident)
        .bind(at)
        .execute(&fx.pool)
        .await
        .unwrap();
}

async fn set_auto_close(fx: &Fx, months: i32) {
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                auto_close_period: Some(Some(months)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_close_closes_stale_issues_and_respects_all_exceptions(pool: PgPool) {
    let fx = setup(pool).await;
    set_auto_close(&fx, 3).await;
    let now = t0() + Duration::days(200);
    let old = t0() - Duration::days(30);
    let mk = |t: &'static str| {
        let app = fx.app.clone();
        async move {
            app.create_issue(&alice(), create("ENG", t))
                .await
                .unwrap()
                .issue
                .identifier
        }
    };
    let stale = mk("Veraltet").await;
    let in_cycle = mk("Im aktiven Cycle").await;
    let in_project = mk("Im laufenden Projekt").await;
    let future_due = mk("Mit künftigem Fälligkeitsdatum").await;
    let with_sla = mk("Mit aktivem SLA").await;
    let parent = mk("Parent mit offenem Child").await;
    let started = mk("Gestartet").await;
    let fresh = mk("Frisch").await;
    let child = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                parent: Some(parent.clone()),
                ..create("ENG", "Child gestartet")
            },
        )
        .await
        .unwrap()
        .issue
        .identifier;
    fx.app
        .update_issue(&alice(), to_state(&child, "In Progress"))
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&started, "In Progress"))
        .await
        .unwrap();
    // Cycle 2 ist zu `now` längst abgeschlossen; für die Ausnahme brauchen wir einen aktiven Cycle zu `now`.
    sqlx::query(
        "UPDATE issue SET cycle_id = (SELECT id FROM cycle WHERE team_id = $1 AND number = 2) WHERE identifier = $2",
    )
    .bind(fx.data.eng.0)
    .bind(&in_cycle)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE cycle SET starts_at = starts_at + interval '5 years', ends_at = ends_at + interval '5 years' WHERE team_id = $1 AND number <> 2")
        .bind(fx.data.eng.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cycle SET starts_at = $2, ends_at = $3, completed_at = NULL WHERE team_id = $1 AND number = 2")
        .bind(fx.data.eng.0)
        .bind(now - Duration::days(1))
        .bind(now + Duration::days(13))
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET project_id = '9201ec70-0000-7000-8000-000000000001' WHERE identifier = $1")
        .bind(&in_project)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET due_date = $2 WHERE identifier = $1")
        .bind(&future_due)
        .bind((now + Duration::days(10)).date_naive())
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET sla_breaches_at = $2, sla_type = 'all', sla_started_at = $3 WHERE identifier = $1")
        .bind(&with_sla)
        .bind(now + Duration::days(2))
        .bind(old)
        .execute(&fx.pool)
        .await
        .unwrap();
    for i in [
        &stale,
        &in_cycle,
        &in_project,
        &future_due,
        &with_sla,
        &parent,
        &started,
        &child,
    ] {
        age(&fx, i, old).await;
    }
    age(&fx, &fresh, now - Duration::days(1)).await;

    let stats = fx.app.run_auto_close(now).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1, "nur das veraltete Issue ohne Ausnahme");
    assert_eq!(state_of(&fx, &stale).await, StateType::Canceled);
    for kept in [
        &in_cycle,
        &in_project,
        &future_due,
        &with_sla,
        &parent,
        &fresh,
    ] {
        assert_eq!(
            state_of(&fx, kept).await,
            StateType::Backlog,
            "{kept} darf nicht geschlossen werden"
        );
    }
    assert_eq!(
        state_of(&fx, &started).await,
        StateType::Started,
        "started wird nie auto-geschlossen"
    );
    let closed = fx.app.get_issue(&alice(), &stale).await.unwrap().view.issue;
    assert_eq!(closed.auto_closed_at, Some(now));
    assert!(closed.canceled_at.is_some());
    let flag: i64 = count(
        &fx.pool,
        "SELECT count(*) FROM issue_history WHERE 'autoClosed' = ANY(flags) AND source = 'automation' AND actor_id IS NULL",
    )
    .await;
    assert_eq!(flag, 1);
    // idempotent
    assert_eq!(fx.app.run_auto_close(now).await.unwrap().processed, 0);

    // Ausnahme-Aufschub endet: Due-Date vergangen, SLA abgelaufen, Child geschlossen
    let later = now + Duration::days(30);
    sqlx::query("UPDATE issue SET project_id = NULL WHERE identifier = $1")
        .bind(&in_project)
        .execute(&fx.pool)
        .await
        .unwrap();
    for i in [&future_due, &with_sla, &in_project] {
        age(&fx, i, old).await;
    }
    sqlx::query("UPDATE issue SET state_id = (SELECT id FROM workflow_state WHERE team_id = $1 AND type = 'canceled' LIMIT 1) WHERE identifier = $2")
        .bind(fx.data.eng.0)
        .bind(&child)
        .execute(&fx.pool)
        .await
        .unwrap();
    age(&fx, &parent, old).await;
    let stats = fx.app.run_auto_close(later).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    for closed in [&future_due, &with_sla, &in_project, &parent] {
        assert_eq!(
            state_of(&fx, closed).await,
            StateType::Canceled,
            "{closed} jetzt schließbar"
        );
    }
    // Der Job-Wrapper läuft ebenfalls.
    AutoCloseJob.run(&fx.app, later).await.unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_close_skips_unfinished_project_but_not_a_finished_one(pool: PgPool) {
    let fx = setup(pool).await;
    set_auto_close(&fx, 1).await;
    let now = t0() + Duration::days(100);
    let a = fx
        .app
        .create_issue(&alice(), create("ENG", "A"))
        .await
        .unwrap()
        .issue
        .identifier;
    sqlx::query("UPDATE issue SET project_id = '9201ec70-0000-7000-8000-000000000001', updated_at = $2 WHERE identifier = $1")
        .bind(&a)
        .bind(t0())
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(fx.app.run_auto_close(now).await.unwrap().processed, 0);
    sqlx::query("UPDATE project SET status_id = (SELECT id FROM project_status WHERE type = 'completed' LIMIT 1)")
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(fx.app.run_auto_close(now).await.unwrap().processed, 1);
    assert_eq!(state_of(&fx, &a).await, StateType::Canceled);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_close_is_off_without_a_period(pool: PgPool) {
    let fx = setup(pool).await;
    let a = fx
        .app
        .create_issue(&alice(), create("ENG", "A"))
        .await
        .unwrap()
        .issue
        .identifier;
    age(&fx, &a, t0() - Duration::days(900)).await;
    assert_eq!(fx.app.run_auto_close(t0()).await.unwrap().processed, 0);
    assert_eq!(state_of(&fx, &a).await, StateType::Backlog);
}

// --- Auto-Archive und Purge ---------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_archive_and_purge(pool: PgPool) {
    let fx = setup(pool).await;
    let a = fx
        .app
        .create_issue(&alice(), create("ENG", "Erledigt"))
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&a.issue.identifier, "Done"))
        .await
        .unwrap();
    let open_parent = fx
        .app
        .create_issue(&alice(), create("ENG", "Offener Parent"))
        .await
        .unwrap();
    let blocked = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                parent: Some(open_parent.issue.identifier.clone()),
                ..create("ENG", "Erledigtes Kind")
            },
        )
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&blocked.issue.identifier, "Done"))
        .await
        .unwrap();
    for i in [&a.issue.identifier, &blocked.issue.identifier] {
        sqlx::query("UPDATE issue SET updated_at = $2, completed_at = $2 WHERE identifier = $1")
            .bind(i)
            .bind(t0() - Duration::days(1))
            .execute(&fx.pool)
            .await
            .unwrap();
    }
    // Default: 6 Monate
    assert_eq!(
        fx.app
            .run_auto_archive(t0() + Duration::days(100))
            .await
            .unwrap()
            .processed,
        0
    );
    let later = t0() + Duration::days(200);
    let stats = fx.app.run_auto_archive(later).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1, "das Kind eines offenen Parents bleibt");
    let archived = fx
        .app
        .get_issue(&alice(), &a.issue.identifier)
        .await
        .unwrap()
        .view
        .issue;
    assert_eq!(archived.archived_at, Some(later));
    let auto: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT auto_archived_at FROM issue WHERE identifier = $1")
            .bind(&a.issue.identifier)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(auto, Some(later));
    assert_eq!(fx.app.run_auto_archive(later).await.unwrap().processed, 0);
    AutoArchiveJob.run(&fx.app, later).await.unwrap();

    // Purge: Papierkorb nach 30 Tagen, inkl. abhängiger Daten
    let trash = fx
        .app
        .create_issue(&alice(), create("ENG", "Weg damit"))
        .await
        .unwrap();
    fx.app
        .create_comment(
            &alice(),
            CreateCommentInput {
                parent: Some(ParentSpec::Issue(trash.issue.identifier.clone())),
                body: "Kommentar zum Löschen".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO attachment (organization_id, issue_id, url, title) VALUES ($1, $2, 'https://example.com/x', 'X')",
    )
    .bind(fx.data.org.0)
    .bind(trash.issue.id.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO document_content (organization_id, owner_type, owner_id, markdown) VALUES ($1, 'issue', $2, 'x')",
    )
    .bind(fx.data.org.0)
    .bind(trash.issue.id.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    fx.app
        .delete_issue(&alice(), &trash.issue.identifier, false)
        .await
        .unwrap();
    let trashed_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT trashed_at FROM issue WHERE id = $1")
            .bind(trash.issue.id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    // 29 Tage: bleibt
    assert_eq!(
        fx.app
            .run_purge(trashed_at + Duration::days(29))
            .await
            .unwrap()
            .processed,
        0
    );
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM issue WHERE title = 'Weg damit'"
        )
        .await,
        1
    );
    let stats = fx
        .app
        .run_purge(trashed_at + Duration::days(31))
        .await
        .unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1);
    for sql in [
        "SELECT count(*) FROM issue WHERE title = 'Weg damit'",
        "SELECT count(*) FROM comment WHERE body LIKE '%Kommentar zum Löschen%'",
        "SELECT count(*) FROM attachment WHERE url = 'https://example.com/x'",
        "SELECT count(*) FROM document_content WHERE owner_type = 'issue' AND markdown = 'x'",
    ] {
        assert_eq!(count(&fx.pool, sql).await, 0, "{sql}");
    }
    // nicht gelöschte Issues bleiben
    assert_eq!(count(&fx.pool, "SELECT count(*) FROM issue").await, 3);
    PurgeJob
        .run(&fx.app, trashed_at + Duration::days(40))
        .await
        .unwrap();
}

// --- Triage-Verantwortung und Rotation -----------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn rotation_switches_the_shift_and_responsibility_assigns_new_triage_issues(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let (schedule, _entries) = fx
        .app
        .create_time_schedule(
            &alice(),
            "ENG Triage",
            vec![
                ScheduleEntryInput {
                    user: Some("Alice Example".into()),
                    user_email: None,
                    starts_at: t0() - Duration::days(1),
                    ends_at: t0() + Duration::days(6),
                    overridden_user: None,
                },
                ScheduleEntryInput {
                    user: Some("Bob Example".into()),
                    user_email: None,
                    starts_at: t0() + Duration::days(6),
                    ends_at: t0() + Duration::days(13),
                    overridden_user: None,
                },
            ],
        )
        .await
        .unwrap();
    // `alice` ist ein Anzeigename; falls die Auflösung scheitert, bricht create_time_schedule oben ab.
    let tr = fx
        .app
        .set_triage_responsibility(
            &alice(),
            SetTriageResponsibilityInput {
                team: "ENG".into(),
                action: Some(TriageAction::Assign),
                users: vec![],
                schedule: Some(schedule.id.to_string()),
            },
        )
        .await
        .unwrap();
    assert!(tr.current_user_id.is_none());
    let info = fx
        .app
        .get_triage_responsibility(&bob(), "ENG", None, 10)
        .await
        .unwrap();
    assert_eq!(info.action, Some(TriageAction::Assign));
    assert_eq!(
        info.current_user.as_ref().map(|u| u.id),
        Some(fx.data.alice)
    );
    assert_eq!(info.upcoming_shifts.len(), 2);
    // Job speichert den aktuellen Verantwortlichen genau einmal
    let stored = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT current_user_id FROM triage_responsibility")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    TriageRotationJob.run(&fx.app, t0()).await.unwrap();
    assert_eq!(stored(fx.pool.clone()).await, Some(fx.data.alice.0));
    let version: i32 = sqlx::query_scalar("SELECT version FROM triage_responsibility")
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    TriageRotationJob
        .run(&fx.app, t0() + Duration::hours(1))
        .await
        .unwrap();
    let again: i32 = sqlx::query_scalar("SELECT version FROM triage_responsibility")
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(version, again, "ohne Wechsel wird nichts gespeichert");
    // Schichtwechsel
    TriageRotationJob
        .run(&fx.app, t0() + Duration::days(7))
        .await
        .unwrap();
    assert_eq!(stored(fx.pool.clone()).await, Some(fx.data.bob.0));
    // Nach dem Ende aller Schichten: niemand
    TriageRotationJob
        .run(&fx.app, t0() + Duration::days(20))
        .await
        .unwrap();
    assert_eq!(stored(fx.pool.clone()).await, None);
    // Nur Schichten eines Users
    let mine = fx
        .app
        .get_triage_responsibility(&bob(), "ENG", Some("Bob Example"), 10)
        .await
        .unwrap();
    assert_eq!(mine.upcoming_shifts.len(), 1);

    // Neue Triage-Issues: `assign` weist dem aktuellen Verantwortlichen zu (Schicht ist zu `t0` vorbei,
    // die App-Uhr steht aber bei `t0`: Alice).
    let id = triage_issue(&fx, "Wird zugewiesen").await;
    let v = fx.app.get_issue(&bob(), &id).await.unwrap().view.issue;
    assert_eq!(v.assignee_id, Some(fx.data.alice));
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM issue_history WHERE 'triageResponsibilityAutoAssigned' = ANY(flags)"
        )
        .await,
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn responsibility_notifies_manual_users_round_robin_assign(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    // Fehlerfälle: keine/zwei Quellen, Unbekannter, Lese-Token, Nicht-Mitglied
    assert_eq!(
        kind(
            fx.app
                .set_triage_responsibility(
                    &alice(),
                    SetTriageResponsibilityInput {
                        team: "ENG".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .set_triage_responsibility(
                    &RequestCtx {
                        scopes: Scopes::read_only(),
                        ..alice()
                    },
                    SetTriageResponsibilityInput {
                        team: "ENG".into(),
                        users: vec!["bob".into()],
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            fx.app
                .set_triage_responsibility(
                    &carol(),
                    SetTriageResponsibilityInput {
                        team: "DES".into(),
                        users: vec!["bob".into()],
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // notify: Verantwortliche bekommen eine Notification beim Eintritt in Triage
    fx.app
        .set_triage_responsibility(
            &alice(),
            SetTriageResponsibilityInput {
                team: "ENG".into(),
                action: Some(TriageAction::Notify),
                users: vec!["Bob Example".into()],
                schedule: None,
            },
        )
        .await
        .unwrap();
    let i = fx
        .app
        .create_issue(&alice(), create("ENG", "Nachricht"))
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&i.issue.identifier, "triage"))
        .await
        .unwrap();
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM notification WHERE type = 'triageResponsibilityIssueAddedToTriage'"
        )
        .await,
        1
    );
    // assign mit zwei Personen: Round-Robin
    fx.app
        .set_triage_responsibility(
            &alice(),
            SetTriageResponsibilityInput {
                team: "ENG".into(),
                action: Some(TriageAction::Assign),
                users: vec!["Bob Example".into(), "Alice Example".into()],
                schedule: None,
            },
        )
        .await
        .unwrap();
    let mut assignees = Vec::new();
    for t in ["Eins", "Zwei", "Drei"] {
        let x = fx
            .app
            .create_issue(&alice(), create("ENG", t))
            .await
            .unwrap();
        let v = fx
            .app
            .update_issue(&alice(), to_state(&x.issue.identifier, "triage"))
            .await
            .unwrap();
        assignees.push(v.issue.assignee_id.unwrap());
    }
    assert_eq!(
        assignees,
        vec![fx.data.bob, fx.data.alice, fx.data.bob],
        "Round-Robin über assignmentIndex"
    );
}

// --- SLA ----------------------------------------------------------------------------------

async fn events(pool: &PgPool, ty: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE payload->>'type' = $1")
        .bind(ty)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sla_job_computes_thresholds_and_reports_each_stage_once(pool: PgPool) {
    let fx = setup(pool).await;
    let i = fx
        .app
        .create_issue(&alice(), create("ENG", "Mit SLA"))
        .await
        .unwrap();
    let breach = t0() + Duration::days(3);
    sqlx::query("UPDATE issue SET sla_breaches_at = $2, sla_type = 'all', sla_started_at = $3 WHERE id = $1")
        .bind(i.issue.id.0)
        .bind(breach)
        .bind(t0())
        .execute(&fx.pool)
        .await
        .unwrap();
    SlaJob
        .run(&fx.app, t0() + Duration::minutes(1))
        .await
        .unwrap();
    let (medium, high): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT sla_medium_risk_at, sla_high_risk_at FROM issue WHERE id = $1")
            .bind(i.issue.id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(high, Some(breach - Duration::hours(24)));
    assert_eq!(medium, Some(t0()), "nie vor dem Start der Uhr");
    assert_eq!(events(&fx.pool, "IssueSlaSet").await, 1);
    assert_eq!(events(&fx.pool, "IssueSlaHighRisk").await, 0);
    // 24 h vor dem Breach: hohes Risiko, einmalig
    let high_at = breach - Duration::hours(23);
    SlaJob.run(&fx.app, high_at).await.unwrap();
    SlaJob
        .run(&fx.app, high_at + Duration::minutes(5))
        .await
        .unwrap();
    assert_eq!(events(&fx.pool, "IssueSlaHighRisk").await, 1);
    assert_eq!(events(&fx.pool, "IssueSlaBreached").await, 0);
    // Breach, einmalig
    SlaJob
        .run(&fx.app, breach + Duration::minutes(1))
        .await
        .unwrap();
    SlaJob
        .run(&fx.app, breach + Duration::minutes(9))
        .await
        .unwrap();
    assert_eq!(events(&fx.pool, "IssueSlaBreached").await, 1);
    assert_eq!(events(&fx.pool, "IssueSlaSet").await, 1);
    assert_eq!(
        count(
            &fx.pool,
            "SELECT count(*) FROM issue_history WHERE 'slaBreached' = ANY(flags)"
        )
        .await,
        1
    );
    // Geschlossene Issues werden nicht mehr gemeldet
    let j = fx
        .app
        .create_issue(&alice(), create("ENG", "Fertig"))
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET sla_breaches_at = $2, sla_type = 'all', sla_started_at = $3 WHERE id = $1")
        .bind(j.issue.id.0)
        .bind(breach)
        .bind(t0())
        .execute(&fx.pool)
        .await
        .unwrap();
    fx.app
        .update_issue(&alice(), to_state(&j.issue.identifier, "Done"))
        .await
        .unwrap();
    SlaJob
        .run(&fx.app, breach + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(events(&fx.pool, "IssueSlaSet").await, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sla_rules_set_and_remove_slas_by_priority(pool: PgPool) {
    let fx = setup(pool).await;
    sqlx::query("UPDATE team SET sla_enabled = true WHERE id = $1")
        .bind(fx.data.eng.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    let rule = |pos: f64, filter: serde_json::Value, action: &str, duration: Option<&str>| {
        let pool = fx.pool.clone();
        let org = fx.data.org.0;
        let team = fx.data.eng.0;
        let action = action.to_owned();
        let duration = duration.map(str::to_owned);
        async move {
            sqlx::query(
                "INSERT INTO sla_rule (organization_id, team_id, position, filter, action, duration, sla_type, name) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'all', 'r')",
            )
            .bind(org)
            .bind(team)
            .bind(pos)
            .bind(filter)
            .bind(action)
            .bind(duration)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    rule(
        1.0,
        serde_json::json!({"priority": {"in": [1]}}),
        "set",
        Some("PT24H"),
    )
    .await;
    rule(
        2.0,
        serde_json::json!({"priority": {"in": [2]}}),
        "set",
        Some("P1W"),
    )
    .await;
    rule(
        3.0,
        serde_json::json!({"priority": {"in": [0, 3, 4]}}),
        "remove",
        None,
    )
    .await;
    let urgent = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                priority: Some(1),
                ..create("ENG", "Dringend")
            },
        )
        .await
        .unwrap();
    let none = fx
        .app
        .create_issue(&alice(), create("ENG", "Ohne"))
        .await
        .unwrap();
    SlaJob
        .run(&fx.app, t0() + Duration::minutes(1))
        .await
        .unwrap();
    let u = fx
        .app
        .get_issue(&alice(), &urgent.issue.identifier)
        .await
        .unwrap()
        .view
        .issue;
    assert_eq!(
        u.sla_breaches_at,
        Some(urgent.issue.created_at + Duration::hours(24))
    );
    assert!(u.sla_high_risk_at.is_some() && u.sla_medium_risk_at.is_some());
    assert_eq!(u.sla_type.map(|t| t.as_str()), Some("all"));
    let n = fx
        .app
        .get_issue(&alice(), &none.issue.identifier)
        .await
        .unwrap()
        .view
        .issue;
    assert!(n.sla_breaches_at.is_none());
    // Statuswechsel auf andere Priorität: Regel 2 ersetzt; Entfernen-Regel nimmt SLA weg.
    let v = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                priority: Some(2),
                ..update(&urgent.issue.identifier)
            },
        )
        .await
        .unwrap();
    let _ = v;
    SlaJob
        .run(&fx.app, t0() + Duration::minutes(2))
        .await
        .unwrap();
    let u = fx
        .app
        .get_issue(&alice(), &urgent.issue.identifier)
        .await
        .unwrap()
        .view
        .issue;
    assert_eq!(
        u.sla_breaches_at,
        Some(urgent.issue.created_at + Duration::days(7))
    );
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                priority: Some(4),
                ..update(&urgent.issue.identifier)
            },
        )
        .await
        .unwrap();
    SlaJob
        .run(&fx.app, t0() + Duration::minutes(3))
        .await
        .unwrap();
    let u = fx
        .app
        .get_issue(&alice(), &urgent.issue.identifier)
        .await
        .unwrap()
        .view
        .issue;
    assert!(u.sla_breaches_at.is_none(), "Entfernen-Regel greift");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn unsnooze_and_delete_responsibility(pool: PgPool) {
    let fx = setup(pool).await;
    enable_triage(&fx, &alice(), "ENG").await;
    let id = triage_issue(&fx, "Kurz weg").await;
    fx.app
        .triage_snooze(
            &bob(),
            TriageSnoozeInput {
                issue: id.clone(),
                until: t0() + Duration::days(2),
                source: HistorySource::User,
            },
        )
        .await
        .unwrap();
    let v = fx
        .app
        .triage_unsnooze(&bob(), &id, HistorySource::User)
        .await
        .unwrap();
    assert!(v.issue.snoozed_until_at.is_none());
    fx.app
        .set_triage_responsibility(
            &alice(),
            SetTriageResponsibilityInput {
                team: "ENG".into(),
                action: None,
                users: vec!["Bob Example".into()],
                schedule: None,
            },
        )
        .await
        .unwrap();
    assert!(
        fx.app
            .delete_triage_responsibility(&alice(), "ENG")
            .await
            .unwrap()
    );
    assert!(
        !fx.app
            .delete_triage_responsibility(&alice(), "ENG")
            .await
            .unwrap()
    );
}
