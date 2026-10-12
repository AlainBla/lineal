#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP34: Planungs-Automationen gegen echte Datenbank (Seed `acme`): Triage-Snooze bei Aktivität,
//! Vorwarnung/Benachrichtigung zum Auto-Archiv, Auto-Archiv von Cycles und Projekten, Kapazität
//! und `cycleNumberBase`.

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::usecases::comment::{CreateCommentInput, ParentSpec};
use lineal_app::usecases::cycle::UpdateCycleInput;
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::team::UpdateTeamInput;
use lineal_app::usecases::triage::TriageSnoozeInput;
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::StateType;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::auto_archive::AutoArchiveJob;
use lineal_testkit::seed::planning::{seed_cycle_id, seed_project_id};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
}

async fn setup(pool: &PgPool) -> Arc<App> {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}

async fn triage_issue(app: &App, title: &str) -> String {
    app.update_team(
        &alice(),
        "ENG",
        UpdateTeamInput {
            triage_enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.create_issue(
        &alice(),
        CreateIssueInput {
            team: "ENG".into(),
            title: Some(title.into()),
            state: Some("triage".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .issue
    .identifier
}

async fn snooze(app: &App, ctx: &RequestCtx, ident: &str) {
    app.triage_snooze(
        ctx,
        TriageSnoozeInput {
            issue: ident.into(),
            until: t0() + Duration::days(3),
            source: HistorySource::User,
        },
    )
    .await
    .unwrap();
}

async fn snoozed_until(pool: &PgPool, ident: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT snoozed_until_at FROM issue WHERE identifier = $1")
        .bind(ident)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comment_by_someone_else_ends_triage_snooze(pool: PgPool) {
    let app = setup(&pool).await;
    let id = triage_issue(&app, "Gesnoozt").await;
    snooze(&app, &bob(), &id).await;
    assert!(snoozed_until(&pool, &id).await.is_some());

    // Kommentar des Snoozenden selbst: Snooze bleibt.
    let comment = |ctx: RequestCtx, body: &str| {
        let app = app.clone();
        let id = id.clone();
        let body = body.to_owned();
        async move {
            app.create_comment(
                &ctx,
                CreateCommentInput {
                    parent: Some(ParentSpec::Issue(id)),
                    body,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    };
    comment(bob(), "Notiz von Bob").await;
    assert!(snoozed_until(&pool, &id).await.is_some());

    // Kommentar eines anderen: das Issue taucht wieder auf.
    comment(alice(), "Neue Information").await;
    assert!(snoozed_until(&pool, &id).await.is_none());
    let by: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT snoozed_by_id FROM issue WHERE identifier = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(by.is_none());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn field_change_by_someone_else_ends_triage_snooze(pool: PgPool) {
    let app = setup(&pool).await;
    let id = triage_issue(&app, "Gesnoozt 2").await;
    snooze(&app, &bob(), &id).await;
    // Änderung des Snoozenden: bleibt gesnoozt.
    app.update_issue(
        &bob(),
        UpdateIssueInput {
            id: id.clone(),
            title: Some("Gesnoozt 2b".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(snoozed_until(&pool, &id).await.is_some());
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: id.clone(),
            title: Some("Gesnoozt 2c".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(snoozed_until(&pool, &id).await.is_none());
    assert_eq!(
        app.get_issue(&alice(), &id)
            .await
            .unwrap()
            .view
            .issue
            .state_type,
        StateType::Triage
    );
}

async fn closed_issue(app: &App, title: &str) -> String {
    let i = app
        .create_issue(
            &alice(),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some(title.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: i.issue.identifier.clone(),
            state: Some("Done".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    i.issue.identifier
}

async fn notifications_of(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notification WHERE type = $1")
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_archive_warns_once_then_archives_and_notifies_creator(pool: PgPool) {
    let app = setup(&pool).await;
    // Ersteller ist Alice; die Job-Benachrichtigung hat keinen Actor, Self-Suppression greift nicht.
    let ident = closed_issue(&app, "Alt").await;
    // Die Periode (6 Monate) reicht von t0 bis 2026-04-10; Abschluss 3 Tage danach: Archivierung in
    // 3 Tagen.
    let closed_at = Utc.with_ymd_and_hms(2026, 4, 13, 7, 15, 0).unwrap();
    sqlx::query("UPDATE issue SET updated_at = $2, completed_at = $2 WHERE identifier = $1")
        .bind(&ident)
        .bind(closed_at)
        .execute(&pool)
        .await
        .unwrap();

    let stats = app.run_auto_archive_warnings(t0()).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1);
    assert_eq!(notifications_of(&pool, "issueAutoArchiveWarning").await, 1);
    // Nur einmal.
    assert_eq!(
        app.run_auto_archive_warnings(t0()).await.unwrap().processed,
        0
    );
    assert_eq!(notifications_of(&pool, "issueAutoArchiveWarning").await, 1);
    // Noch nicht archiviert.
    assert_eq!(app.run_auto_archive(t0()).await.unwrap().processed, 0);

    // Vier Tage später ist die Frist erreicht: archiviert, Ersteller wird benachrichtigt.
    let later = t0() + Duration::days(4);
    AutoArchiveJob.run(&app, later).await.unwrap();
    let archived: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM issue WHERE identifier = $1")
            .bind(&ident)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archived, Some(later));
    assert_eq!(notifications_of(&pool, "issueAutoArchived").await, 1);
    // Wiederholung erzeugt nichts Neues.
    AutoArchiveJob.run(&app, later).await.unwrap();
    assert_eq!(notifications_of(&pool, "issueAutoArchived").await, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_archive_covers_cycles_and_projects(pool: PgPool) {
    let app = setup(&pool).await;
    let eng = SeedData::acme().eng.0;
    // Das Seed-Projekt abschließen und von Issues befreien.
    let project = seed_project_id().0;
    sqlx::query(
        "UPDATE project SET updated_at = $2, status_id = \
         (SELECT id FROM project_status WHERE type = 'completed' LIMIT 1) WHERE id = $1",
    )
    .bind(project)
    .bind(t0() - Duration::days(1))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE issue SET project_id = NULL, project_milestone_id = NULL WHERE project_id = $1",
    )
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    // Ein Issue im abgeschlossenen Seed-Cycle 1.
    let ident = closed_issue(&app, "Im alten Cycle").await;
    sqlx::query("UPDATE issue SET cycle_id = $2 WHERE identifier = $1")
        .bind(&ident)
        .bind(seed_cycle_id(1).0)
        .execute(&pool)
        .await
        .unwrap();

    // Nach 3 Monaten (Periode 6): nichts.
    let early = t0() + Duration::days(90);
    let s = app.run_planning_auto_archive(early).await.unwrap();
    assert!(s.errors.is_empty(), "{:?}", s.errors);
    assert_eq!(s.processed, 0);

    let later = t0() + Duration::days(200);
    let s = app.run_planning_auto_archive(later).await.unwrap();
    assert!(s.errors.is_empty(), "{:?}", s.errors);
    assert_eq!(s.processed, 2, "ein Cycle und ein Projekt");
    let (arch, auto): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT archived_at, auto_archived_at FROM cycle WHERE id = $1")
            .bind(seed_cycle_id(1).0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((arch, auto), (Some(later), Some(later)));
    let cycle_of_issue: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT cycle_id FROM issue WHERE identifier = $1")
            .bind(&ident)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(cycle_of_issue.is_none(), "Issues werden vom Cycle gelöst");
    let (parch, pauto): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT archived_at, auto_archived_at FROM project WHERE id = $1")
            .bind(project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((parch, pauto), (Some(later), Some(later)));
    // Die laufenden/künftigen Cycles bleiben unberührt, ein zweiter Lauf ändert nichts.
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cycle WHERE team_id = $1 AND archived_at IS NOT NULL",
    )
    .bind(eng)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(open, 1);
    assert_eq!(
        app.run_planning_auto_archive(later)
            .await
            .unwrap()
            .processed,
        0
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_with_open_issue_is_not_auto_archived(pool: PgPool) {
    let app = setup(&pool).await;
    let project = seed_project_id().0;
    sqlx::query(
        "UPDATE project SET updated_at = $2, status_id = \
         (SELECT id FROM project_status WHERE type = 'completed' LIMIT 1) WHERE id = $1",
    )
    .bind(project)
    .bind(t0() - Duration::days(1))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE issue SET project_id = NULL, project_milestone_id = NULL WHERE project_id = $1",
    )
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    let open = app
        .create_issue(
            &alice(),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Noch offen".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET project_id = $2 WHERE identifier = $1")
        .bind(&open.issue.identifier)
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
    app.run_planning_auto_archive(t0() + Duration::days(200))
        .await
        .unwrap();
    let archived: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM project WHERE id = $1")
            .bind(project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(archived.is_none());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn capacity_uses_velocity_of_past_cycles(pool: PgPool) {
    let app = setup(&pool).await;
    // Ohne Historie: Mitglieder × defaultIssueEstimate × cycleDuration.
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM team_membership WHERE team_id = $1")
            .bind(SeedData::acme().eng.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (team, views) = app.list_cycles(&alice(), "ENG", None).await.unwrap();
    let est = f64::from(team.default_issue_estimate) * f64::from(team.cycle_duration);
    let upcoming = views.iter().find(|v| v.is_future).unwrap();
    assert_eq!(upcoming.capacity, Some(members as f64 * est));
    let done = views.iter().find(|v| v.is_past).unwrap();
    assert_eq!(
        done.capacity, None,
        "abgeschlossene Cycles haben keine Schätzung"
    );

    // Mit Historie: Mittel der letzten Werte.
    sqlx::query("UPDATE cycle SET completed_scope_history = '{2,5,8}' WHERE id = $1")
        .bind(seed_cycle_id(1).0)
        .execute(&pool)
        .await
        .unwrap();
    let (_, views) = app.list_cycles(&alice(), "ENG", None).await.unwrap();
    let upcoming = views.iter().find(|v| v.is_future).unwrap();
    assert_eq!(upcoming.capacity, Some(8.0));
    let active = views.iter().find(|v| v.is_active).unwrap();
    assert_eq!(active.capacity, Some(8.0));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn rename_with_trailing_number_renumbers_following_cycles(pool: PgPool) {
    let app = setup(&pool).await;
    let eng = SeedData::acme().eng.0;
    // Seed: #1 beendet, #2 aktiv, #3 und #4 künftig. Der aktive Cycle wird "Sprint 40".
    app.update_cycle(
        &alice(),
        UpdateCycleInput {
            id: seed_cycle_id(2).0.to_string(),
            name: Some(Some("Sprint 40".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let numbers: Vec<(i32, Option<String>)> =
        sqlx::query_as("SELECT number, name FROM cycle WHERE team_id = $1 ORDER BY starts_at")
            .bind(eng)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        numbers.iter().map(|n| n.0).collect::<Vec<_>>(),
        vec![1, 2, 41, 42]
    );
    let (base, last): (Option<i32>, i32) =
        sqlx::query_as("SELECT cycle_number_base, last_cycle_number FROM team WHERE id = $1")
            .bind(eng)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((base, last), (Some(40), 42));

    // Namen ohne Zahl am Ende ändern nichts.
    app.update_cycle(
        &alice(),
        UpdateCycleInput {
            id: seed_cycle_id(2).0.to_string(),
            name: Some(Some("Fokus".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let again: Vec<i32> =
        sqlx::query_scalar("SELECT number FROM cycle WHERE team_id = $1 ORDER BY starts_at")
            .bind(eng)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(again, vec![1, 2, 41, 42]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn rename_to_a_colliding_number_is_rejected(pool: PgPool) {
    let app = setup(&pool).await;
    // "Sprint 1" würde #3 -> 2 nummerieren; 2 gehört dem aktiven Cycle.
    let err = app
        .update_cycle(
            &alice(),
            UpdateCycleInput {
                id: seed_cycle_id(2).0.to_string(),
                name: Some(Some("Sprint 1".into())),
                ..Default::default()
            },
        )
        .await
        .err()
        .expect("collision");
    assert_eq!(err.kind, lineal_app::ErrorKind::InvalidRequest);
}
