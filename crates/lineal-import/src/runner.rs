//! Ausführung reservierter Läufe (Job `import_runner` und CLI): Quelle aus dem verschlüsselten
//! Token bauen, scannen oder importieren, Fehler ohne Zugangsdaten festhalten.

use crate::adapter::linear::LinearSource;
use crate::blob::AppBlobSink;
use crate::engine::{ImportEngine, RunOutcome};
use chrono::{DateTime, Duration, Utc};
use lineal_app::App;
use lineal_app::error::{AppError, AppResult};
use lineal_app::ports::repos::import::{ImportRun, ImportRunUpdate};
use lineal_domain::ids::OrgId;
use uuid::Uuid;

/// Wie lange ein Runner einen Lauf exklusiv hält, ohne die Lease zu erneuern.
pub const LEASE: Duration = Duration::minutes(2);

/// Nach dieser Zeit ohne Lebenszeichen gilt ein CLI-Lauf als abgebrochen (Zombie).
pub const STALE_AFTER: Duration = Duration::minutes(30);

/// Markiert einen Lauf als fehlgeschlagen (Meldung ohne Zugangsdaten).
pub async fn fail_run(app: &App, org: OrgId, run_id: Uuid, message: &str) -> AppResult<()> {
    let now = app.clock.now();
    let mut tx = app.store.begin(org).await?;
    tx.imports()
        .update_import_run(
            run_id,
            &ImportRunUpdate {
                status: Some("failed".into()),
                error: Some(Some(message.chars().take(500).collect())),
                finished_at: Some(Some(now)),
                now: Some(now),
                ..Default::default()
            },
        )
        .await?;
    tx.imports().set_import_lease(run_id, None).await?;
    tx.commit().await
}

/// Führt einen reservierten Lauf aus (`scanning` -> Scan, `running` -> Import).
pub async fn run_claimed(app: &App, run: &ImportRun) -> AppResult<()> {
    let org = OrgId(run.org_id);
    if run.source != "linear" {
        return fail_run(
            app,
            org,
            run.id,
            "runs of this source are executed by the command line",
        )
        .await;
    }
    let Some(token) = app.import_load_token((org, run.id)).await? else {
        return fail_run(
            app,
            org,
            run.id,
            "the stored credentials are no longer available; start a new scan",
        )
        .await;
    };
    let source = match LinearSource::from_env(&token) {
        Ok(s) => s,
        Err(e) => return fail_run(app, org, run.id, &e.to_string()).await,
    };
    let sink = AppBlobSink { app };
    let engine = ImportEngine::new(app, &source, &sink);
    match run.status.as_str() {
        "scanning" => engine.scan(org, run.id).await,
        "running" => match engine.run(org, run.id).await {
            Ok(RunOutcome::Failed(_)) | Ok(_) => Ok(()),
            Err(e) => fail_run(app, org, run.id, &e.message).await,
        },
        _ => Ok(()),
    }
}

/// Ein Takt des Runners: abgelaufene Zugangsdaten löschen, einen Lauf reservieren und ausführen.
/// Liefert `true`, wenn ein Lauf bearbeitet wurde.
pub async fn tick(app: &App, now: DateTime<Utc>) -> AppResult<bool> {
    let mut tx = app.store.begin_system().await?;
    let purged = tx.imports().delete_expired_import_credentials(now).await?;
    if purged > 0 {
        tracing::info!(purged, "expired import credentials deleted");
    }
    let reaped = tx
        .imports()
        .fail_stale_import_runs(now, now - STALE_AFTER)
        .await?;
    if reaped > 0 {
        tracing::warn!(reaped, "interrupted import runs marked as failed");
    }
    let claimed = tx.imports().claim_runnable_import(now, now + LEASE).await?;
    tx.commit().await?;
    match claimed {
        Some(run) => {
            tracing::info!(run = %run.id, status = %run.status, "import runner picked up a run");
            if let Err(e) = run_claimed(app, &run).await {
                tracing::error!(run = %run.id, error = %e.message, "import run failed unexpectedly");
                let _ = fail_run(app, OrgId(run.org_id), run.id, &e.message).await;
            }
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Kurzform für Fehler aus Eingaben (CLI).
pub fn invalid(msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg)
}
