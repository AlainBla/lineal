//! De-privilegierte Anwendungsrolle `lineal_app` (LIN-61 / S-09).
//!
//! Hintergrund (S-09): Server und Worker verbinden sich bisher als Eigentuemer/Superuser, und keine
//! Tabelle hat Row-Level-Security – die Mandantentrennung haengt allein am Anwendungscode. Als
//! Defense-in-Depth verbinden sich Server und Worker mit dieser Rolle, die nur DML auf dem
//! App-Schema `public` darf (SELECT/INSERT/UPDATE/DELETE, Sequenzen, Funktionen) – kein DDL, kein
//! Superuser, keine Ownership. Migrationen laufen weiter als Eigentuemer (`lineal migrate`), der die
//! Rolle einmalig anlegt und berechtigt ([`ensure_app_role`]).
//!
//! RLS ist bewusst NICHT Teil von LIN-61 (eigener Folge-Task, siehe
//! `docs/superpowers/specs/2026-11-16-rls-folgeschritt.md`); die Rolle traegt dafuer bereits
//! `NOBYPASSRLS`.

use sqlx::postgres::{PgConnectOptions, PgPool};

/// Name der de-privilegierten Rolle, mit der sich Server und Worker verbinden.
pub const APP_ROLE: &str = "lineal_app";

/// Leitet aus den Verbindungsoptionen des Eigentuemers die Optionen fuer die App-Rolle ab:
/// gleicher Host/Port/Datenbank, aber Benutzer `lineal_app` und das uebergebene Passwort.
pub fn app_role_options(base: PgConnectOptions, password: &str) -> PgConnectOptions {
    base.username(APP_ROLE).password(password)
}

fn is_duplicate_object(err: &sqlx::Error) -> bool {
    // 42710 = duplicate_object (Rolle existiert bereits; moegliches Rennen paralleler `migrate`-Laeufe).
    err.as_database_error().and_then(|e| e.code()).as_deref() == Some("42710")
}

/// Legt die Rolle `lineal_app` an (bzw. aktualisiert Passwort und Attribute bei bestehender Rolle)
/// und erteilt ihr genau die noetigen DML-Rechte auf das App-Schema `public`.
///
/// Laeuft als Eigentuemer/Superuser (aus `lineal migrate`), ist idempotent und sicher auf einer
/// bestehenden Datenbank mit Daten. Erteilt bewusst NICHT: DDL, `CREATE`, Ownership, Superuser.
/// Kuenftige vom migrierenden Eigentuemer angelegte Objekte deckt `ALTER DEFAULT PRIVILEGES` ab, so
/// dass neue Migrationen keine Anpassung brauchen.
pub async fn ensure_app_role(pool: &PgPool, password: &str) -> Result<(), sqlx::Error> {
    // DO-Bloecke und DDL nehmen keine Bind-Parameter; `format(..., %L, $1)` baut das Statement
    // serverseitig und escaped das Passwort injektionssicher (quote_literal-Semantik).
    let create_stmt: String = sqlx::query_scalar(
        "SELECT format('CREATE ROLE lineal_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOREPLICATION NOBYPASSRLS PASSWORD %L', $1)",
    )
    .bind(password)
    .fetch_one(pool)
    .await?;
    let alter_stmt: String = sqlx::query_scalar(
        "SELECT format('ALTER ROLE lineal_app WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOREPLICATION NOBYPASSRLS PASSWORD %L', $1)",
    )
    .bind(password)
    .fetch_one(pool)
    .await?;

    // Anlegen oder (bestehende Rolle / Rennen) Passwort und Attribute aktualisieren.
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(APP_ROLE)
            .fetch_one(pool)
            .await?;
    if exists {
        sqlx::query(&alter_stmt).execute(pool).await?;
    } else if let Err(err) = sqlx::query(&create_stmt).execute(pool).await {
        if is_duplicate_object(&err) {
            sqlx::query(&alter_stmt).execute(pool).await?;
        } else {
            return Err(err);
        }
    }

    // Rechte auf das bestehende Schema `public` (nur DML; Sequenzen fuer nextval, Funktionen fuer
    // Trigger/Helper). Erneutes GRANT ist idempotent.
    for stmt in [
        "GRANT USAGE ON SCHEMA public TO lineal_app",
        "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO lineal_app",
        "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO lineal_app",
        "GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO lineal_app",
        // Kuenftige Objekte des migrierenden Eigentuemers automatisch nutzbar machen (ohne
        // `FOR ROLE`: gilt fuer die aktuell migrierende Rolle, egal wie sie heisst).
        "ALTER DEFAULT PRIVILEGES IN SCHEMA public \
         GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO lineal_app",
        "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO lineal_app",
        "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT EXECUTE ON FUNCTIONS TO lineal_app",
    ] {
        sqlx::query(stmt).execute(pool).await?;
    }

    Ok(())
}
