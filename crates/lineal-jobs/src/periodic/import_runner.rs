//! Periodischer Job `import_runner` (WP18): führt Scan- und Importläufe aus (`lineal-import`) und
//! löscht abgelaufene Token-Zugangsdaten (7 Tage). Läufe tragen eine Lease, sodass mehrere
//! Worker oder die CLI denselben Lauf nie gleichzeitig bearbeiten.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct ImportRunnerJob;

#[async_trait]
impl PeriodicJob for ImportRunnerJob {
    fn name(&self) -> &'static str {
        "import_runner"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(10))
    }

    /// Ein Import dauert Stunden: eigene Task, der Worker-Takt bleibt frei (B7.02).
    fn detached(&self) -> bool {
        true
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        // Ein Lauf je Takt; langlaufende Importe halten ihre Lease selbst.
        lineal_import::runner::tick(app, now).await.map(|_| ())
    }
}
