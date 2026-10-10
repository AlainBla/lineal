//! Worker-Schleife: Outbox-Relay, Einplanen periodischer Jobs, Abarbeiten der Job-Queue.

use crate::{EventConsumer, PeriodicJob, Schedule, registry};
use chrono::Duration as ChronoDuration;
use lineal_app::App;
use lineal_app::AppResult;
use lineal_app::ports::repos::NewJob;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

const PERIODIC_PREFIX: &str = "periodic:";

/// Einstellungen des Workers.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub worker_id: String,
    /// Maximale Anzahl Events bzw. Jobs pro Durchlauf.
    pub batch: i64,
    /// Wartezeit, wenn nichts zu tun war.
    pub poll_interval: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", std::process::id()),
            batch: 100,
            poll_interval: Duration::from_millis(200),
        }
    }
}

/// Was ein Durchlauf getan hat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickStats {
    /// Zustellungen an Konsumenten.
    pub deliveries: usize,
    /// Ausgeführte Jobs.
    pub jobs_run: usize,
}

impl TickStats {
    fn idle(&self) -> bool {
        self.deliveries == 0 && self.jobs_run == 0
    }
}

pub struct Worker {
    app: Arc<App>,
    consumers: Vec<Arc<dyn EventConsumer>>,
    periodic: Vec<Arc<dyn PeriodicJob>>,
    cfg: WorkerConfig,
}

impl Worker {
    /// Worker mit allen Konsumenten und Jobs aus der Registry.
    pub fn new(app: Arc<App>, cfg: WorkerConfig) -> Self {
        Self::with(app, registry::consumers(), registry::periodic_jobs(), cfg)
    }

    /// Worker mit expliziter Liste (Tests).
    pub fn with(
        app: Arc<App>,
        consumers: Vec<Arc<dyn EventConsumer>>,
        periodic: Vec<Arc<dyn PeriodicJob>>,
        cfg: WorkerConfig,
    ) -> Self {
        Self {
            app,
            consumers,
            periodic,
            cfg,
        }
    }

    /// Ein Durchlauf: Outbox verteilen, periodische Jobs einplanen, fällige Jobs ausführen.
    pub async fn tick(&self) -> AppResult<TickStats> {
        let deliveries = self.relay_outbox().await?;
        self.schedule_periodic().await?;
        let jobs_run = self.run_jobs().await?;
        Ok(TickStats {
            deliveries,
            jobs_run,
        })
    }

    /// Läuft bis `shutdown` fertig ist.
    pub async fn run(&self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let idle = match self.tick().await {
                Ok(stats) => stats.idle(),
                Err(e) => {
                    tracing::error!(error = %e, "worker tick failed");
                    true
                }
            };
            if idle {
                tokio::select! {
                    () = &mut shutdown => break,
                    () = tokio::time::sleep(self.cfg.poll_interval) => {}
                }
            } else {
                // Es gab Arbeit: sofort weiter, aber ein bereits gesetztes Shutdown beachten.
                tokio::select! {
                    biased;
                    () = &mut shutdown => break,
                    () = std::future::ready(()) => {}
                }
            }
        }
    }

    async fn relay_outbox(&self) -> AppResult<usize> {
        if self.consumers.is_empty() {
            return Ok(0);
        }
        let mut delivered = 0;
        for consumer in &self.consumers {
            let mut tx = self.app.store.begin_system().await?;
            let events = tx
                .outbox()
                .claim_undelivered(consumer.name(), self.cfg.batch)
                .await?;
            let mut ok_ids = Vec::with_capacity(events.len());
            for ev in &events {
                match consumer.handle(&self.app, ev).await {
                    Ok(()) => ok_ids.push(ev.id),
                    Err(e) => tracing::warn!(
                        consumer = consumer.name(), event = %ev.id, error = %e,
                        "consumer failed, event stays undelivered"
                    ),
                }
            }
            tx.outbox().mark_delivered(consumer.name(), &ok_ids).await?;
            tx.commit().await?;
            delivered += ok_ids.len();
        }
        let names: Vec<String> = self.consumers.iter().map(|c| c.name().to_owned()).collect();
        let mut tx = self.app.store.begin_system().await?;
        tx.outbox().mark_processed(&names).await?;
        tx.commit().await?;
        Ok(delivered)
    }

    async fn schedule_periodic(&self) -> AppResult<()> {
        if self.periodic.is_empty() {
            return Ok(());
        }
        let now = self.app.clock.now();
        let mut tx = self.app.store.begin_system().await?;
        for job in &self.periodic {
            let Schedule::Every(every) = job.schedule();
            let secs = i64::try_from(every.as_secs()).unwrap_or(i64::MAX).max(1);
            let slot = now.timestamp().div_euclid(secs);
            let mut new = NewJob::now(
                format!("{PERIODIC_PREFIX}{}", job.name()),
                serde_json::json!({}),
                now,
            );
            new.unique_key = Some(format!("{PERIODIC_PREFIX}{}:{slot}", job.name()));
            new.max_attempts = 3;
            tx.jobs().enqueue(new).await?;
        }
        tx.commit().await
    }

    async fn run_jobs(&self) -> AppResult<usize> {
        let now = self.app.clock.now();
        let mut tx = self.app.store.begin_system().await?;
        let claimed = tx
            .jobs()
            .claim(&self.cfg.worker_id, self.cfg.batch, now)
            .await?;
        tx.commit().await?;

        for job in &claimed {
            let outcome = match job.kind.strip_prefix(PERIODIC_PREFIX) {
                Some(name) => match self.periodic.iter().find(|p| p.name() == name) {
                    Some(p) => p
                        .run(&self.app, now)
                        .await
                        .map_err(|e| (e.to_string(), true)),
                    None => Err((format!("unknown periodic job `{name}`"), false)),
                },
                None => Err((format!("no handler for job kind `{}`", job.kind), false)),
            };
            let mut tx = self.app.store.begin_system().await?;
            match outcome {
                Ok(()) => tx.jobs().complete(job.id, self.app.clock.now()).await?,
                Err((msg, retry)) => {
                    tracing::warn!(job = %job.id, kind = %job.kind, error = %msg, "job failed");
                    let backoff = ChronoDuration::seconds(60 * i64::from(job.attempts.max(1)));
                    let retry_at = retry.then(|| self.app.clock.now() + backoff);
                    tx.jobs().fail(job.id, &msg, retry_at).await?;
                }
            }
            tx.commit().await?;
        }
        Ok(claimed.len())
    }
}
