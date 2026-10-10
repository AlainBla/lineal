//! Schnittstellen für Outbox-Konsumenten und periodische Jobs.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};
use std::time::Duration;

/// Verarbeitet Outbox-Events idempotent (at-least-once, Schlüssel = `StoredEvent::id`).
/// Zustellung wird je Konsument in `outbox_delivery` vermerkt.
#[async_trait]
pub trait EventConsumer: Send + Sync {
    /// Eindeutiger Name (Schlüssel in `outbox_delivery.consumer`).
    fn name(&self) -> &'static str;
    async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()>;
}

/// Zeitplan eines periodischen Jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// Einmal pro Zeitfenster dieser Länge (mehrere Worker führen es trotzdem nur einmal aus).
    Every(Duration),
}

/// Periodischer Job; läuft über die Tabelle `job` mit Sperre, damit mehrere Worker sicher sind.
#[async_trait]
pub trait PeriodicJob: Send + Sync {
    /// Eindeutiger Name (Job-Art `periodic:<name>`).
    fn name(&self) -> &'static str;
    fn schedule(&self) -> Schedule;
    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()>;
}
