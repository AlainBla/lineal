//! `PgStore` und `PgTx`. Gemeinsame Datei: nur S1 / Orchestrator.

use crate::db_err;
use async_trait::async_trait;
use lineal_app::AppResult;
use lineal_app::ports::repos::*;
use lineal_app::ports::{Store, Tx};
use lineal_domain::ids::OrgId;
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Transaction};

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
        let tx = self.pool.begin().await.map_err(db_err)?;
        Ok(Box::new(PgTx { tx, org, sync_id: None }))
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

    async fn commit(self: Box<Self>) -> AppResult<()> {
        self.tx.commit().await.map_err(db_err)
    }
}
