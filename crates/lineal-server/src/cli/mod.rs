//! Subcommands des Binaries `lineal`. Gemeinsame Datei: nur S1 / Orchestrator.

pub mod create_workspace;
pub mod import;
pub mod migrate;
pub mod schema_export;
pub mod seed;
pub mod serve;
pub mod totp;
pub mod worker;

use crate::adapters::build_app;
use crate::config::Config;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lineal_app::App;
use lineal_db::PgStore;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
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
    /// TOTP (2FA) verwalten, z. B. zuruecksetzen (LIN-70).
    Totp(totp::TotpArgs),
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
        Command::Totp(args) => totp::run(args).await,
    }
}

/// Verbindung, Store und `App` aus der Umgebung. `enforce_statement_timeout` gilt nur für den
/// Request-bedienenden Pool (serve); Migrationen, Import und Worker verbinden ohne `statement_timeout`.
///
/// `use_app_role` (LIN-61/S-09): `true` für die Anwendung (serve, worker) – ist
/// `LINEAL_APP_DB_PASSWORD` gesetzt, verbindet der Pool als de-privilegierte Rolle `lineal_app`
/// (nur DML). `false` für Migrationen und Betreiber-Befehle (migrate, seed, create-workspace,
/// import, totp): sie verbinden als Eigentuemer aus `DATABASE_URL`. Ohne gesetztes Passwort fällt
/// auch die Anwendung auf die Eigentuemer-URL zurück (Dev/Test, bestehende Installationen).
pub(crate) async fn connect(
    enforce_statement_timeout: bool,
    use_app_role: bool,
) -> Result<(Config, PgPool, Arc<App>)> {
    let config = Config::from_env()?;
    let statement_timeout = enforce_statement_timeout
        .then_some(config.db_statement_timeout_ms)
        .flatten();
    let mut options: PgConnectOptions = config
        .database_url
        .parse()
        .context("DATABASE_URL is not a valid PostgreSQL connection URL")?;
    if use_app_role {
        match config.app_db_password.as_deref() {
            Some(password) => options = lineal_db::app_role_options(options, password),
            // Stiller Rückfall auf den Eigentuemer wäre in Prod die Art, wie S-09 ein Jahr lang
            // ungedeployt bliebe – daher ein deutlicher Hinweis (nicht im Dev-Modus).
            None if !config.dev => tracing::warn!(
                "connecting as the database owner; set LINEAL_APP_DB_PASSWORD to use the \
                 de-privileged `{}` role (LIN-61/S-09)",
                lineal_db::APP_ROLE
            ),
            None => {}
        }
    }
    let pool = lineal_db::connect_opts(options, config.db_max_connections, statement_timeout)
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
