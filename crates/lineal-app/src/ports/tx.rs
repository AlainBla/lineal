//! `Store` und `Tx`: Transaktionsgrenze mit Zugriff auf alle Repositories.
//! Gemeinsame Datei: nur S1 / Orchestrator.

use super::repos::*;
use crate::error::AppResult;
use async_trait::async_trait;
use lineal_domain::ids::OrgId;

/// Einstieg in die Persistenz.
#[async_trait]
pub trait Store: Send + Sync {
    /// Beginnt eine Transaktion im Kontext eines Workspace.
    async fn begin(&self, org: OrgId) -> AppResult<Box<dyn Tx>>;
    /// Beginnt eine mandantenübergreifende Transaktion (nur Outbox-Relay und Job-Worker).
    async fn begin_system(&self) -> AppResult<Box<dyn Tx>>;
}

/// Eine Datenbank-Transaktion mit Zugriff auf alle Repositories.
/// Wird sie ohne `commit` verworfen, wird zurückgerollt.
#[async_trait]
pub trait Tx: Send {
    /// Workspace dieser Transaktion (`OrgId::nil()` bei `begin_system`).
    fn org_id(&self) -> OrgId;
    /// Workspaceweit monotone `lastSyncId` dieser Transaktion (wird beim ersten Aufruf
    /// aus der Sequenz `sync_seq` gezogen und danach wiederverwendet).
    async fn last_sync_id(&mut self) -> AppResult<i64>;

    fn orgs(&mut self) -> &mut dyn OrgRepo;
    fn users(&mut self) -> &mut dyn UserRepo;
    fn auth(&mut self) -> &mut dyn AuthRepo;
    fn teams(&mut self) -> &mut dyn TeamRepo;
    fn states(&mut self) -> &mut dyn WorkflowStateRepo;
    fn labels(&mut self) -> &mut dyn LabelRepo;
    fn issues(&mut self) -> &mut dyn IssueRepo;
    fn comments(&mut self) -> &mut dyn CommentRepo;
    fn attachments(&mut self) -> &mut dyn AttachmentRepo;
    fn cycles(&mut self) -> &mut dyn CycleRepo;
    fn projects(&mut self) -> &mut dyn ProjectRepo;
    fn initiatives(&mut self) -> &mut dyn InitiativeRepo;
    fn views(&mut self) -> &mut dyn ViewRepo;
    fn notifications(&mut self) -> &mut dyn NotificationRepo;
    fn documents(&mut self) -> &mut dyn DocumentRepo;
    fn templates(&mut self) -> &mut dyn TemplateRepo;
    fn triage(&mut self) -> &mut dyn TriageRepo;
    fn webhooks(&mut self) -> &mut dyn WebhookRepo;
    fn outbox(&mut self) -> &mut dyn OutboxRepo;
    fn jobs(&mut self) -> &mut dyn JobRepo;
    /// Zusatz gegenüber docs/11: Kennzahlen für `get_workspace_overview` (WP11).
    fn overview(&mut self) -> &mut dyn OverviewRepo;
    /// Zusatz gegenüber docs/11: Import-Repository (WP18).
    fn imports(&mut self) -> &mut dyn ImportRepo;
    /// Zusatz gegenüber docs/11: Audit-Log (WP25, append-only).
    fn audit(&mut self) -> &mut dyn AuditRepo;

    async fn commit(self: Box<Self>) -> AppResult<()>;
}
