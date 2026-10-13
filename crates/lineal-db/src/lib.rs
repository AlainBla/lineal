//! PostgreSQL-Implementierung der Ports aus `lineal-app`.
#![forbid(unsafe_code)]

pub mod filter_sql;
pub mod repos;
pub mod search;
pub mod store;

pub use store::{PgStore, PgTx};

use lineal_app::AppError;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Eingebettete Migrationen (`crates/lineal-db/migrations`).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Öffnet einen Verbindungspool.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Wendet alle ausstehenden Migrationen an.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// Bildet einen sqlx-Fehler auf `AppError::internal` ab (Details nur im Log).
///
/// Ausnahme (B8.02): NUL-Zeichen in Texten (SQLSTATE 22021 bzw. 22P05 bei JSON) sind ein
/// Eingabefehler des Aufrufers und werden zu `invalid_request` statt `internal`.
pub(crate) fn db_err(e: sqlx::Error) -> AppError {
    if let Some(code) = e.as_database_error().and_then(|d| d.code())
        && matches!(code.as_ref(), "22021" | "22P05")
    {
        return AppError::invalid_request("Text must not contain NUL characters.");
    }
    tracing::error!(error = %e, "database error");
    AppError::internal("database error")
}
