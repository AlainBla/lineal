//! F.10 „Remind me“: Setzen, Job, Sichtbarkeit (Seed `acme`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{Duration, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::ids::*;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::reminders::RemindersJob;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: &PgPool) -> (Arc<App>, SeedData, chrono::DateTime<Utc>) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let now = Utc
        .with_ymd_and_hms(2026, 10, 10, 8, 0, 0)
        .single()
        .unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(lineal_testkit::fake::FixedClock(now));
    (Arc::new(app), SeedData::acme(), now)
}

async fn issue(pool: &PgPool, d: &SeedData, team: TeamId, slot: u128, number: i32) -> IssueId {
    let id = IssueId::new_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
         VALUES ($1,$2,$3,$4,'Login schlägt fehl',$5,$6)",
    )
    .bind(id.0)
    .bind(d.org.0)
    .bind(team.0)
    .bind(number)
    .bind(seed_workflow_state_ids(slot)[1].0)
    .bind(d.alice.0)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn kinds(pool: &PgPool, user: UserId) -> Vec<String> {
    sqlx::query_scalar("SELECT type FROM notification WHERE user_id = $1 ORDER BY created_at, id")
        .bind(user.0)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reminder_fires_once_and_can_be_reset(pool: PgPool) {
    let (app, d, now) = setup(&pool).await;
    let issue = issue(&pool, &d, d.eng, 1, 931).await;
    let bob = ctx_for(SeedUser::Bob);
    let target = issue.0.to_string();

    // Vergangenheit und zu ferne Zukunft werden abgelehnt.
    let err = app
        .set_reminder(&bob, "issue", &target, Some(now - Duration::hours(1)))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert!(
        app.set_reminder(&bob, "issue", &target, Some(now + Duration::days(4000)))
            .await
            .is_err()
    );

    let r = app
        .set_reminder(&bob, "issue", &target, Some(now + Duration::hours(1)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.user_id, d.bob);
    // Erneutes Setzen ersetzt (eine Zeile je Objekt).
    app.set_reminder(&bob, "issue", &target, Some(now + Duration::hours(3)))
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM reminder")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    assert!(
        app.get_reminder(&bob, "issue", &target)
            .await
            .unwrap()
            .is_some()
    );

    // Noch nicht fällig.
    assert_eq!(
        app.create_reminder_notifications(now + Duration::hours(2))
            .await
            .unwrap(),
        0
    );
    // Fällig: auch die eigene Aktion erzeugt die Notification (kein Akteur).
    RemindersJob
        .run(&app, now + Duration::hours(4))
        .await
        .unwrap();
    assert_eq!(kinds(&pool, d.bob).await, vec!["issueReminder"]);
    assert!(kinds(&pool, d.alice).await.is_empty());
    // Einmalig.
    assert_eq!(
        app.create_reminder_notifications(now + Duration::hours(5))
            .await
            .unwrap(),
        0
    );
    let item = &app
        .list_notifications(&bob, Default::default())
        .await
        .unwrap()
        .items[0];
    assert_eq!(item.view.n.category, "reminders");
    assert!(
        item.subtitle.starts_with("Reminder for"),
        "{}",
        item.subtitle
    );
    assert!(
        item.view.n.is_priority,
        "reminders gehören zu den Prioritätskategorien"
    );

    // Entfernen vor Fälligkeit: keine Notification.
    app.set_reminder(&bob, "issue", &target, Some(now + Duration::hours(6)))
        .await
        .unwrap();
    app.set_reminder(&bob, "issue", &target, None)
        .await
        .unwrap();
    assert_eq!(
        app.create_reminder_notifications(now + Duration::hours(7))
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reminder_respects_visibility_and_user_scope(pool: PgPool) {
    let (app, d, now) = setup(&pool).await;
    let hidden = issue(&pool, &d, d.des, 3, 932).await;
    let bob = ctx_for(SeedUser::Carol);
    // Privates Team: für Carol nicht sichtbar -> not_found, nie forbidden.
    let err = app
        .set_reminder(
            &bob,
            "issue",
            &hidden.0.to_string(),
            Some(now + Duration::hours(1)),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    for kind in ["project", "initiative", "document"] {
        let err = app
            .set_reminder(
                &bob,
                kind,
                &uuid::Uuid::now_v7().to_string(),
                Some(now + Duration::hours(1)),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound, "{kind}");
    }
    let err = app
        .set_reminder(&bob, "team", "x", Some(now + Duration::hours(1)))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);

    // Alice sieht das Issue; Alices Erinnerung ist für Carol unsichtbar.
    let alice = ctx_for(SeedUser::Bob);
    let target = hidden.0.to_string();
    app.set_reminder(&alice, "issue", &target, Some(now + Duration::hours(1)))
        .await
        .unwrap();
    assert!(app.get_reminder(&bob, "issue", &target).await.is_err());
    // Verliert Alice den Zugriff vor Fälligkeit, entsteht keine Notification.
    sqlx::query("UPDATE issue SET trashed = true WHERE id = $1")
        .bind(hidden.0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.create_reminder_notifications(now + Duration::hours(2))
            .await
            .unwrap(),
        0
    );
    assert!(kinds(&pool, d.bob).await.is_empty());
}
