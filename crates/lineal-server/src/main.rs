//! Binary `lineal`.
#![forbid(unsafe_code)]

use anyhow::Result;
use clap::Parser;
use lineal_server::cli::{Cli, run};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // `.env` ist optional (Entwicklung); echte Umgebungsvariablen haben Vorrang.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    run(Cli::parse()).await
}
