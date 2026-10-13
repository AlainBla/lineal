//! Worker-Schleife: Outbox-Relay, Einplanen periodischer Jobs, Abarbeiten der Job-Queue.

use crate::{EventConsumer, PeriodicJob, Schedule, registry};
use chrono::Duration as ChronoDuration;
use lineal_app::App;
use lineal_app::AppResult;
use lineal_app::ports::repos::NewJob;
use lineal_domain::ids::JobId;
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex};
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
    /// Fehlversuche eines Konsumenten je Event, danach gilt das Event als zugestellt (mit
    /// Fehler-Log, "Dead Letter"), damit ein kaputtes Event die Warteschlange nicht dauerhaft belastet.
    pub max_event_attempts: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", std::process::id()),
            batch: 100,
            poll_interval: Duration::from_millis(200),
            max_event_attempts: 10,
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
    /// Namen der gerade laufenden abgekoppelten Jobs (höchstens eine Instanz je Name).
    running_detached: Arc<Mutex<HashSet<&'static str>>>,
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
            running_detached: Arc::default(),
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
                    Err(e) => {
                        // Fehlversuche stehen in `outbox_delivery` und überleben einen Neustart.
                        let attempts = tx
                            .outbox()
                            .record_failure(consumer.name(), ev.id, &e.to_string())
                            .await?;
                        let limit = i32::try_from(self.cfg.max_event_attempts).unwrap_or(i32::MAX);
                        if attempts >= limit {
                            tracing::error!(
                                consumer = consumer.name(), event = %ev.id, error = %e,
                                attempts,
                                "consumer gave up on event (dead letter), marked as delivered"
                            );
                            tx.outbox().mark_dead_letter(consumer.name(), ev.id).await?;
                            delivered += 1;
                        } else {
                            tracing::warn!(
                                consumer = consumer.name(), event = %ev.id, error = %e, attempts,
                                "consumer failed, event stays undelivered"
                            );
                        }
                    }
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
            let periodic = job
                .kind
                .strip_prefix(PERIODIC_PREFIX)
                .and_then(|name| self.periodic.iter().find(|p| p.name() == name));
            if let Some(p) = periodic.filter(|p| p.detached()) {
                self.spawn_detached(p, job.id, job.attempts, &job.kind, now)
                    .await?;
                continue;
            }
            let outcome = match job.kind.strip_prefix(PERIODIC_PREFIX) {
                Some(name) => match periodic {
                    Some(p) => p
                        .run(&self.app, now)
                        .await
                        .map_err(|e| (e.to_string(), true)),
                    None => Err((format!("unknown periodic job `{name}`"), false)),
                },
                None => Err((format!("no handler for job kind `{}`", job.kind), false)),
            };
            finish_job(&self.app, job.id, job.attempts, &job.kind, outcome).await?;
        }
        Ok(claimed.len())
    }

    /// Startet einen abgekoppelten Job in eigener Task. Läuft bereits eine Instanz, wird der Slot
    /// als erledigt markiert (übersprungen).
    async fn spawn_detached(
        &self,
        job: &Arc<dyn PeriodicJob>,
        id: JobId,
        attempts: i32,
        kind: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let name = job.name();
        let started = self
            .running_detached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name);
        if !started {
            return finish_job(&self.app, id, attempts, kind, Ok(())).await;
        }
        let (app, job, busy, kind) = (
            Arc::clone(&self.app),
            Arc::clone(job),
            Arc::clone(&self.running_detached),
            kind.to_owned(),
        );
        tokio::spawn(async move {
            let outcome = job.run(&app, now).await.map_err(|e| (e.to_string(), true));
            busy.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(name);
            if let Err(e) = finish_job(&app, id, attempts, &kind, outcome).await {
                tracing::error!(job = %id, error = %e, "could not record the job result");
            }
        });
        Ok(())
    }
}

/// Ergebnis eines Jobs in der Queue vermerken (erledigt bzw. fehlgeschlagen mit Backoff).
async fn finish_job(
    app: &App,
    id: JobId,
    attempts: i32,
    kind: &str,
    outcome: Result<(), (String, bool)>,
) -> AppResult<()> {
    let mut tx = app.store.begin_system().await?;
    match outcome {
        Ok(()) => tx.jobs().complete(id, app.clock.now()).await?,
        Err((msg, retry)) => {
            tracing::warn!(job = %id, kind = %kind, error = %msg, "job failed");
            let backoff = ChronoDuration::seconds(60 * i64::from(attempts.max(1)));
            let retry_at = retry.then(|| app.clock.now() + backoff);
            tx.jobs().fail(id, &msg, retry_at).await?;
        }
    }
    tx.commit().await
}
