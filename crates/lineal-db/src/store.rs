//! `PgStore` und `PgTx`. Gemeinsame Datei: nur S1 / Orchestrator.

use crate::db_err;
use async_trait::async_trait;
use lineal_app::ports::repos::*;
use lineal_app::ports::{Store, Tx};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::OrgId;
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Transaction};

/// Wiederholt eine komplette Transaktion bei Deadlock (40P01) bzw. Serialisierungsfehler (40001);
/// siehe [`lineal_app::error::with_retry`].
pub use lineal_app::error::with_retry;

/// Zentrale Fehlerabbildung für Datenbankfehler:
/// - 40P01 (Deadlock) und 40001 (Serialisierung): `conflict` mit `details.retryable = true`
///   (wird von [`with_retry`] wiederholt; nach erschöpften Versuchen so gemeldet, nie `internal`);
/// - 23503 (Fremdschlüssel): `not_found`, wenn das referenzierte Objekt fehlt bzw. inzwischen
///   gelöscht wurde, `conflict`, wenn der Datensatz noch referenziert wird;
/// - 23505 (Unique) und 23P01 (Exclusion): `conflict`;
/// - alles andere: `internal` (Details nur im Log).
pub fn map_db_error(e: &sqlx::Error) -> AppError {
    let db = e.as_database_error();
    match db.and_then(|d| d.code()).as_deref() {
        Some(code @ ("40P01" | "40001")) => AppError::retryable_conflict(code),
        Some("23503") => {
            let msg = db.map(|d| d.message()).unwrap_or_default();
            if msg.contains("update or delete on table") {
                AppError::conflict("The object is still referenced by other objects.")
            } else {
                AppError::not_found("A referenced object does not exist (anymore).")
            }
        }
        Some("23505") => AppError::conflict("An object with these values already exists."),
        Some("23P01") => AppError::conflict("The change conflicts with an existing object."),
        // B8.02: NUL-Zeichen in Texten (22021, bei JSON 22P05) sind ein Eingabefehler.
        Some("22021" | "22P05") => {
            AppError::invalid_request("Text must not contain NUL characters.")
        }
        _ => {
            tracing::error!(error = %e, "database error");
            AppError::internal("database error")
        }
    }
}

/// Postgres-Implementierung von [`Store`].
#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Zugrunde liegender Pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn begin_tx(&self, org: OrgId) -> AppResult<Box<dyn Tx>> {
        // `Transaction::begin` (sqlx 0.8) ist nicht abbruchsicher: wird die Future nach dem Senden
        // von `BEGIN`, aber vor der Antwort verworfen (Client trennt, Timeout), geht die Verbindung
        // "idle in transaction" in den Pool zurück (Notice "there is already a transaction in
        // progress" beim nächsten Nutzer, Schreibzugriffe landen in einer nie committeten
        // Transaktion; B1.12). Deshalb läuft das Öffnen in einer eigenen Task, die der Aufrufer
        // nicht abbrechen kann; wird er abgebrochen, rollt die Task die Transaktion beim Drop zurück.
        let pool = self.pool.clone();
        let tx = tokio::spawn(async move { pool.begin().await })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "begin task failed");
                AppError::internal("database error")
            })?
            .map_err(db_err)?;
        Ok(Box::new(PgTx {
            tx,
            org,
            sync_id: None,
        }))
    }
}

#[async_trait]
impl Store for PgStore {
    async fn begin(&self, org: OrgId) -> AppResult<Box<dyn Tx>> {
        self.begin_tx(org).await
    }

    async fn begin_system(&self) -> AppResult<Box<dyn Tx>> {
        self.begin_tx(OrgId::nil()).await
    }
}

/// Eine Postgres-Transaktion; implementiert alle Repo-Traits (je Datei in `repos/`).
pub struct PgTx {
    pub(crate) tx: Transaction<'static, Postgres>,
    pub(crate) org: OrgId,
    pub(crate) sync_id: Option<i64>,
}

impl PgTx {
    /// Siehe [`Tx::advisory_lock`].
    pub(crate) async fn advisory_lock_inner(
        &mut self,
        scope: &str,
        key: Option<uuid::Uuid>,
    ) -> AppResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "lineal:{}:{scope}:{}",
                self.org.0,
                key.map(|k| k.to_string()).unwrap_or_default()
            ))
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// Zieht beim ersten Aufruf `nextval('sync_seq')` und liefert danach denselben Wert.
    pub(crate) async fn last_sync_id_inner(&mut self) -> AppResult<i64> {
        if let Some(id) = self.sync_id {
            return Ok(id);
        }
        let id: i64 = sqlx::query_scalar("SELECT nextval('sync_seq')")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)?;
        self.sync_id = Some(id);
        Ok(id)
    }
}

#[async_trait]
impl Tx for PgTx {
    fn org_id(&self) -> OrgId {
        self.org
    }

    async fn last_sync_id(&mut self) -> AppResult<i64> {
        self.last_sync_id_inner().await
    }

    async fn advisory_lock(&mut self, scope: &str, key: Option<uuid::Uuid>) -> AppResult<()> {
        self.advisory_lock_inner(scope, key).await
    }

    fn orgs(&mut self) -> &mut dyn OrgRepo {
        self
    }
    fn users(&mut self) -> &mut dyn UserRepo {
        self
    }
    fn auth(&mut self) -> &mut dyn AuthRepo {
        self
    }
    fn teams(&mut self) -> &mut dyn TeamRepo {
        self
    }
    fn states(&mut self) -> &mut dyn WorkflowStateRepo {
        self
    }
    fn labels(&mut self) -> &mut dyn LabelRepo {
        self
    }
    fn issues(&mut self) -> &mut dyn IssueRepo {
        self
    }
    fn comments(&mut self) -> &mut dyn CommentRepo {
        self
    }
    fn attachments(&mut self) -> &mut dyn AttachmentRepo {
        self
    }
    fn cycles(&mut self) -> &mut dyn CycleRepo {
        self
    }
    fn projects(&mut self) -> &mut dyn ProjectRepo {
        self
    }
    fn initiatives(&mut self) -> &mut dyn InitiativeRepo {
        self
    }
    fn views(&mut self) -> &mut dyn ViewRepo {
        self
    }
    fn notifications(&mut self) -> &mut dyn NotificationRepo {
        self
    }
    fn documents(&mut self) -> &mut dyn DocumentRepo {
        self
    }
    fn templates(&mut self) -> &mut dyn TemplateRepo {
        self
    }
    fn triage(&mut self) -> &mut dyn TriageRepo {
        self
    }
    fn webhooks(&mut self) -> &mut dyn WebhookRepo {
        self
    }
    fn outbox(&mut self) -> &mut dyn OutboxRepo {
        self
    }
    fn jobs(&mut self) -> &mut dyn JobRepo {
        self
    }
    fn overview(&mut self) -> &mut dyn OverviewRepo {
        self
    }
    fn imports(&mut self) -> &mut dyn ImportRepo {
        self
    }
    fn audit(&mut self) -> &mut dyn AuditRepo {
        self
    }

    async fn commit(self: Box<Self>) -> AppResult<()> {
        // Ebenfalls in eigener Task: ein Abbruch des Aufrufers (Request-Abbruch) darf `COMMIT`
        // weder halb senden noch eine bereits begonnene Zusage zurücknehmen.
        let tx = self.tx;
        tokio::spawn(async move { tx.commit().await })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "commit task failed");
                AppError::internal("database error")
            })?
            .map_err(db_err)
    }
}
