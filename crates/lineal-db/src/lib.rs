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

/// Bildet einen sqlx-Fehler zentral auf einen `AppError` ab (siehe [`store::map_db_error`]).
pub(crate) fn db_err(e: sqlx::Error) -> AppError {
    store::map_db_error(&e)
}
