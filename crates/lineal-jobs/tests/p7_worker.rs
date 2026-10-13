#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 7: Worker-Entkopplung vom Import und Aufbewahrung von Job-/Outbox-Tabellen.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::{NewEvent, StoredEvent};
use lineal_app::{App, AppResult};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, OrgId, TeamId};
use lineal_jobs::{EventConsumer, PeriodicJob, Schedule, Worker, WorkerConfig};
use lineal_testkit::test_app;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

struct Counting(Arc<AtomicUsize>);

#[async_trait]
impl EventConsumer for Counting {
    fn name(&self) -> &'static str {
        "counting"
    }
    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Langläufer (wie der Import): blockiert, bis das Tor geöffnet wird.
struct SlowDetached {
    started: Arc<AtomicUsize>,
    gate: Arc<Notify>,
}

#[async_trait]
impl PeriodicJob for SlowDetached {
    fn name(&self) -> &'static str {
        "slow_detached"
    }
    fn schedule(&self) -> Schedule {
        // Ein einziger Slot, unabhängig davon, wann der Test über eine Sekundengrenze läuft.
        Schedule::Every(Duration::from_secs(3600))
    }
    fn detached(&self) -> bool {
        true
    }
    async fn run(&self, _app: &App, _now: DateTime<Utc>) -> AppResult<()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.gate.notified().await;
        Ok(())
    }
}

async fn push_event(app: &App) {
    let mut tx = app.store.begin(OrgId::new_v7()).await.unwrap();
    tx.outbox()
        .push(NewEvent::new(
            DomainEvent::IssueCreated {
                issue_id: IssueId::new_v7(),
                team_id: TeamId::new_v7(),
                mentioned_user_ids: vec![],
            },
            EventOrigin::User,
        ))
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// B7.02: Ein laufender Import (abgekoppelter Job) blockiert den Worker nicht: Outbox-Konsumenten
/// liefern weiter aus, und es startet keine zweite Instanz.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn detached_job_does_not_block_outbox_delivery(pool: PgPool) {
    let app = test_app(Arc::new(PgStore::new(pool.clone())));
    let (started, delivered) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let gate = Arc::new(Notify::new());
    let worker = Worker::with(
        app.clone(),
        vec![Arc::new(Counting(delivered.clone()))],
        vec![Arc::new(SlowDetached {
            started: started.clone(),
            gate: gate.clone(),
        })],
        WorkerConfig::default(),
    );
    push_event(&app).await;
    let stats = tokio::time::timeout(Duration::from_secs(10), worker.tick())
        .await
        .expect("tick must not wait for the detached job")
        .unwrap();
    assert_eq!(stats.deliveries, 1);
    // Der Job läuft (blockiert), die Outbox wird trotzdem weiter abgearbeitet.
    push_event(&app).await;
    let stats = tokio::time::timeout(Duration::from_secs(10), worker.tick())
        .await
        .expect("tick must not wait for the detached job")
        .unwrap();
    assert_eq!(stats.deliveries, 1);
    assert_eq!(delivered.load(Ordering::SeqCst), 2);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    let done = |pool: PgPool| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM job WHERE kind = 'periodic:slow_detached' AND status = 'done'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(done(pool.clone()).await, 0, "the job row stays running");
    // Tor öffnen: der Job endet und vermerkt sein Ergebnis.
    gate.notify_one();
    for _ in 0..100 {
        if done(pool.clone()).await == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the detached job did not finish");
}

/// B7.05: Erledigte Jobs und verarbeitete Outbox-Events werden nach der Aufbewahrungsfrist
/// gelöscht; frische, offene und Dead-Letter-Einträge (bis 30 Tage) bleiben.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn retention_purges_old_finished_jobs_and_processed_events(pool: PgPool) {
    let app = test_app(Arc::new(PgStore::new(pool.clone())));
    for (status, age_days) in [
        ("done", 3),
        ("done", 0),
        ("failed", 20),
        ("failed", 2),
        ("queued", 40),
    ] {
        sqlx::query(
            "INSERT INTO job (id, kind, status, run_at, completed_at) \
             VALUES (gen_random_uuid(), 'k', $1, now() - make_interval(days => $2), \
                     CASE WHEN $1 = 'done' THEN now() - make_interval(days => $2) END)",
        )
        .bind(status)
        .bind(age_days)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Events: (Alter in Tagen, verarbeitet, Dead-Letter)
    for (age, processed, dead) in [
        (10, true, false),
        (10, true, true),
        (40, true, true),
        (1, true, false),
        (10, false, false),
    ] {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO outbox_event (event_id, organization_id, origin, type, payload, sync_id, \
               created_at, processed_at) \
             VALUES ($1, gen_random_uuid(), 'user', 't', '{}', 1, now() - make_interval(days => $2), \
               CASE WHEN $3 THEN now() - make_interval(days => $2) END)",
        )
        .bind(id)
        .bind(age)
        .bind(processed)
        .execute(&pool)
        .await
        .unwrap();
        if dead {
            sqlx::query(
                "INSERT INTO outbox_delivery (consumer, event_id, delivered_at, attempts, last_error) \
                 VALUES ('c', $1, now(), 10, 'boom')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    let (jobs, events) = lineal_jobs::periodic::purge::purge_queues(&app, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(jobs, 2, "old done and old failed jobs");
    assert_eq!(events, 2, "old processed event and the 40-day dead letter");
    let count = |table: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(count("job").await, 3);
    assert_eq!(count("outbox_event").await, 3);
    assert_eq!(
        count("outbox_delivery").await,
        1,
        "deliveries of purged events are gone"
    );
}
