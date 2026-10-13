//! Binary `lineal`.
#![forbid(unsafe_code)]

use anyhow::Result;
use clap::Parser;
use lineal_server::cli::{Cli, run};
use tracing_subscriber::EnvFilter;

/// Stackgröße der Tokio-Worker. Die Use-Case-Futures (Retry, Sperren, Template-Anwendung) sind
/// tief verschachtelt; mit dem Standard von 2 MiB lief der Debug-Build beim Anlegen von Issues
/// über den Stack.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_BYTES)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    // `.env` ist optional (Entwicklung); echte Umgebungsvariablen haben Vorrang.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    run(Cli::parse()).await
}
