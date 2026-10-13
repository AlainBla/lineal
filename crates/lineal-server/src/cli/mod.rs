//! Subcommands des Binaries `lineal`. Gemeinsame Datei: nur S1 / Orchestrator.

pub mod create_workspace;
pub mod import;
pub mod migrate;
pub mod schema_export;
pub mod seed;
pub mod serve;
pub mod worker;

use crate::adapters::build_app;
use crate::config::Config;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lineal_app::App;
use lineal_db::PgStore;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "lineal", version, about = "Lineal – Issue-Tracker-Server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// HTTP-Server starten (GraphQL, MCP, Sync, REST).
    Serve,
    /// Worker starten (Outbox-Konsumenten, periodische Jobs).
    Worker,
    /// Datenbankmigrationen anwenden.
    Migrate,
    /// Beispiel-Workspace `acme` anlegen.
    Seed {
        /// Zusätzlich realistische Demo-Daten für die Web-UI anlegen (Issues, Projekte, Cycle,
        /// Templates, Dokumente).
        #[arg(long)]
        demo: bool,
        /// Seed auch ohne `LINEAL_DEV=1` erlauben (Benutzer mit bekanntem Passwort `lineal-dev`!).
        #[arg(long)]
        allow_insecure_demo_users: bool,
    },
    /// Ersteinrichtung: Workspace und Owner anlegen (Passwort aus LINEAL_OWNER_PASSWORD oder
    /// interaktiv ohne Echo).
    CreateWorkspace(create_workspace::CreateWorkspaceArgs),
    /// Import aus Linear (API oder CSV-Export).
    Import(import::ImportArgs),
    /// GraphQL-Schema als SDL exportieren.
    SchemaExport {
        /// Zieldatei.
        #[arg(long, default_value = "web/schema.graphql")]
        out: PathBuf,
    },
}

/// Führt den Subcommand aus.
pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Serve => serve::run().await,
        Command::Worker => worker::run().await,
        Command::Migrate => migrate::run().await,
        Command::Seed {
            demo,
            allow_insecure_demo_users,
        } => seed::run(demo, allow_insecure_demo_users).await,
        Command::CreateWorkspace(args) => create_workspace::run(args).await,
        Command::Import(args) => import::run(args).await,
        Command::SchemaExport { out } => schema_export::run(&out),
    }
}

/// Verbindung, Store und `App` aus der Umgebung.
pub(crate) async fn connect() -> Result<(Config, PgPool, Arc<App>)> {
    let config = Config::from_env()?;
    let pool = lineal_db::connect(&config.database_url, config.db_max_connections)
        .await
        .context("cannot connect to the database (is `docker compose up -d` running?)")?;
    let app = build_app(Arc::new(PgStore::new(pool.clone())), &config);
    Ok((config, pool, app))
}

/// Wartet auf SIGTERM (`docker stop`, systemd, Kubernetes) oder Ctrl-C/SIGINT.
pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => tracing::info!("received SIGTERM"),
                    r = tokio::signal::ctrl_c() => match r {
                        Ok(()) => tracing::info!("received SIGINT"),
                        Err(e) => {
                            tracing::error!(error = %e, "cannot listen for ctrl-c");
                            std::future::pending::<()>().await;
                        }
                    },
                }
            }
            Err(e) => {
                // Ohne SIGTERM-Handler wäre `docker stop` wieder ein SIGKILL: laut scheitern.
                tracing::error!(error = %e, "cannot install the SIGTERM handler");
                wait_ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    wait_ctrl_c().await;
    tracing::info!("shutting down");
}

async fn wait_ctrl_c() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "cannot listen for ctrl-c");
        std::future::pending::<()>().await;
    }
}

/// Wie lange `serve` nach dem Signal auf laufende Verbindungen wartet (`LINEAL_SHUTDOWN_GRACE_SECS`,
/// Default 10 s; unter dem `stop_grace_period` des Containers). Danach wird beendet, auch wenn
/// langlebige Verbindungen (WebSockets) noch offen sind.
pub(crate) fn shutdown_grace() -> std::time::Duration {
    let secs = std::env::var("LINEAL_SHUTDOWN_GRACE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(10);
    std::time::Duration::from_secs(secs)
}
