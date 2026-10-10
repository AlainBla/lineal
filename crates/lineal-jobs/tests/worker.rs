//! Worker gegen echte Datenbank (Testmuster für Crates außerhalb von `lineal-db`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::{NewEvent, StoredEvent};
use lineal_app::{App, AppResult};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, OrgId, TeamId};
use lineal_jobs::{EventConsumer, PeriodicJob, Schedule, TickStats, Worker, WorkerConfig};
use lineal_testkit::test_app;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Counting(&'static str, Arc<AtomicUsize>);

#[async_trait]
impl EventConsumer for Counting {
    fn name(&self) -> &'static str {
        self.0
    }
    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Ticker(Arc<AtomicUsize>);

#[async_trait]
impl PeriodicJob for Ticker {
    fn name(&self) -> &'static str {
        "ticker"
    }
    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }
    async fn run(&self, _app: &App, _now: DateTime<Utc>) -> AppResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn worker_delivers_each_event_once_per_consumer_and_runs_periodic_once(pool: PgPool) {
    let app = test_app(Arc::new(PgStore::new(pool)));
    let mut tx = app.store.begin(OrgId::new_v7()).await.unwrap();
    for _ in 0..3 {
        let ev = DomainEvent::IssueCreated {
            issue_id: IssueId::new_v7(),
            team_id: TeamId::new_v7(),
        };
        tx.outbox()
            .push(NewEvent::new(ev, EventOrigin::User))
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();

    let (a, b, ticks) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let worker = Worker::with(
        app,
        vec![
            Arc::new(Counting("a", a.clone())),
            Arc::new(Counting("b", b.clone())),
        ],
        vec![Arc::new(Ticker(ticks.clone()))],
        WorkerConfig::default(),
    );

    assert_eq!(
        worker.tick().await.unwrap(),
        TickStats {
            deliveries: 6,
            jobs_run: 1
        }
    );
    assert_eq!(
        worker.tick().await.unwrap(),
        TickStats {
            deliveries: 0,
            jobs_run: 0
        },
        "idempotent, same slot"
    );
    assert_eq!(
        (
            a.load(Ordering::SeqCst),
            b.load(Ordering::SeqCst),
            ticks.load(Ordering::SeqCst)
        ),
        (3, 3, 1)
    );
}
