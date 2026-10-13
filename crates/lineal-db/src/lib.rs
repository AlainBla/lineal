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

/// Prüft beim Entnehmen aus dem Pool, ob die Verbindung sauber ist (B1.12): Eine abgebrochene
/// `BEGIN`-Anfrage (Future während `pool.begin()` verworfen) hinterlässt eine Server-Sitzung mit
/// offener Transaktion, die sqlx nicht kennt (`transaction_depth = 0`); das nächste `BEGIN` meldet
/// dann „there is already a transaction in progress“, und Statements außerhalb von `Tx` liefen in
/// der nie abgeschlossenen Transaktion. `now()` ist der Transaktionsbeginn, `statement_timestamp()`
/// der Beginn dieses Statements (einfaches Protokoll): ungleich = offene Transaktion. Solche
/// Verbindungen werden geschlossen (Server rollt zurück), der Pool nimmt eine andere. Die Abfrage
/// ersetzt zugleich den Standard-Ping von sqlx (`test_before_acquire`).
fn connection_is_clean(conn: &mut sqlx::PgConnection) -> BoxedCheck<'_> {
    use sqlx::Row;
    Box::pin(async move {
        let row = sqlx::Executor::fetch_one(conn, "SELECT now() = statement_timestamp()").await?;
        let clean: bool = row.try_get(0)?;
        if !clean {
            tracing::warn!("closing pooled connection with an open transaction");
        }
        Ok(clean)
    })
}

type BoxedCheck<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, sqlx::Error>> + Send + 'a>>;

/// Pool-Optionen mit Zustandsprüfung (siehe [`connection_is_clean`]).
pub fn pool_options(max_connections: u32) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .test_before_acquire(false)
        .before_acquire(|conn, _meta| connection_is_clean(conn))
}

/// Öffnet einen Verbindungspool.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    pool_options(max_connections).connect(database_url).await
}

/// Wendet alle ausstehenden Migrationen an.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// Bildet einen sqlx-Fehler zentral auf einen `AppError` ab (siehe [`store::map_db_error`]).
pub(crate) fn db_err(e: sqlx::Error) -> AppError {
    store::map_db_error(&e)
}
