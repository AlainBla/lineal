//! Subcommands des Binaries `lineal`. Gemeinsame Datei: nur S1 / Orchestrator.

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
    Seed,
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
        Command::Seed => seed::run().await,
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

/// Wartet auf Ctrl-C.
pub(crate) async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "cannot listen for ctrl-c");
        std::future::pending::<()>().await;
    }
    tracing::info!("shutting down");
}
