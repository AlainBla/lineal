//! `FakeStore`/`FakeTx`: alle Repos als Stubs, `find_principal_by_token_hash` konfigurierbar.
//! Damit lassen sich Auth und Adapter (WP03/WP04) ohne Datenbank und ohne WP02 testen.

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use lineal_app::ports::repos::*;
use lineal_app::ports::{
    BlobStore, Clock, IdGen, Mail, Mailer, PasswordHasher, SearchHit, SearchIndex, Store,
    TotpKeyProvider, Tx,
};
use lineal_app::{App, AppConfig, AppResult};
use lineal_domain::ids::{EventId, JobId, OrgId, UserId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use uuid::Uuid;

/// Recovery-Code-Zeile im Fake: (id, user_id, code_hash, used_at).
type FakeRecoveryRow = (Uuid, Uuid, Vec<u8>, Option<DateTime<Utc>>);

#[derive(Default)]
struct FakeState {
    principals: Mutex<HashMap<String, Principal>>,
    events: Mutex<Vec<NewEvent>>,
    jobs: Mutex<Vec<NewJob>>,
    audit: Mutex<Vec<NewAuditEntry>>,
    sync_seq: AtomicI64,
    totp: Mutex<HashMap<Uuid, UserTotp>>,
    recovery: Mutex<Vec<FakeRecoveryRow>>,
    grace: Mutex<HashMap<Uuid, GraceRow>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Store ohne Datenbank. Klone teilen denselben Zustand.
#[derive(Clone, Default)]
pub struct FakeStore {
    state: Arc<FakeState>,
}

impl FakeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registriert einen Aufrufer für einen Token-Hash (hex), wie ihn WP02 aus API-Keys ableitet.
    pub fn with_principal(self, token_hash: impl Into<String>, principal: Principal) -> Self {
        lock(&self.state.principals).insert(token_hash.into(), principal);
        self
    }

    /// Alle bisher in der Outbox gelandeten Events.
    pub fn pushed_events(&self) -> Vec<NewEvent> {
        lock(&self.state.events).clone()
    }

    /// Alle bisher eingeplanten Jobs.
    pub fn enqueued_jobs(&self) -> Vec<NewJob> {
        lock(&self.state.jobs).clone()
    }

    /// Alle bisher geschriebenen Audit-Einträge (aller Workspaces, in Schreibreihenfolge).
    pub fn audit_entries(&self) -> Vec<NewAuditEntry> {
        lock(&self.state.audit).clone()
    }
}

#[async_trait]
impl Store for FakeStore {
    async fn begin(&self, org: OrgId) -> AppResult<Box<dyn Tx>> {
        Ok(Box::new(FakeTx {
            state: self.state.clone(),
            org,
        }))
    }
    async fn begin_system(&self) -> AppResult<Box<dyn Tx>> {
        self.begin(OrgId::nil()).await
    }
}

/// Transaktion des [`FakeStore`].
pub struct FakeTx {
    state: Arc<FakeState>,
    org: OrgId,
}

macro_rules! empty_repos {
    ($($t:ident),+ $(,)?) => {$( #[async_trait] impl $t for FakeTx {} )+};
}

empty_repos!(
    OrgRepo,
    UserRepo,
    TeamRepo,
    WorkflowStateRepo,
    LabelRepo,
    IssueRepo,
    CommentRepo,
    AttachmentRepo,
    CycleRepo,
    ProjectRepo,
    InitiativeRepo,
    ViewRepo,
    NotificationRepo,
    DocumentRepo,
    TemplateRepo,
    TriageRepo,
    WebhookRepo,
    OverviewRepo,
    ImportRepo,
);

#[async_trait]
impl AuditRepo for FakeTx {
    async fn append(&mut self, entry: &NewAuditEntry) -> AppResult<()> {
        lock(&self.state.audit).push(entry.clone());
        Ok(())
    }
    async fn list(
        &mut self,
        filter: &AuditFilter,
        cursor: Option<AuditCursor>,
        limit: i64,
    ) -> AppResult<Vec<AuditEntry>> {
        let mut rows: Vec<AuditEntry> = lock(&self.state.audit)
            .iter()
            .filter(|e| e.organization_id == self.org)
            .filter(|e| filter.action.as_deref().is_none_or(|a| a == e.action))
            .filter(|e| {
                filter
                    .actor_user_id
                    .is_none_or(|u| Some(u) == e.actor_user_id)
            })
            .filter(|e| filter.since.is_none_or(|t| e.occurred_at >= t))
            .filter(|e| filter.until.is_none_or(|t| e.occurred_at < t))
            .map(|e| AuditEntry {
                id: e.id,
                organization_id: e.organization_id,
                occurred_at: e.occurred_at,
                actor_user_id: e.actor_user_id,
                actor_kind: e.actor_kind.as_str().to_owned(),
                credential_kind: e.credential_kind.map(str::to_owned),
                action: e.action.clone(),
                target_type: e.target_type.clone(),
                target_id: e.target_id,
                details: e.details.clone(),
                ip: e.ip.map(|i| i.to_string()),
                request_id: e.request_id.clone(),
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse((r.occurred_at, r.id)));
        if let Some(c) = cursor {
            rows.retain(|r| (r.occurred_at, r.id) < (c.occurred_at, c.id));
        }
        rows.truncate(usize::try_from(limit).unwrap_or(0));
        Ok(rows)
    }
}

#[async_trait]
impl AuthRepo for FakeTx {
    async fn find_principal_by_token_hash(
        &mut self,
        hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<Principal>> {
        Ok(lock(&self.state.principals).get(hash).cloned())
    }
}

#[async_trait]
impl TotpRepo for FakeTx {
    async fn get(&mut self, user: UserId) -> AppResult<Option<UserTotp>> {
        Ok(lock(&self.state.totp).get(&user.0).cloned())
    }
    async fn upsert_pending(&mut self, rec: &NewUserTotp) -> AppResult<()> {
        lock(&self.state.totp).insert(
            rec.user_id.0,
            UserTotp {
                user_id: rec.user_id,
                secret_enc: rec.secret_enc.clone(),
                dek_wrapped: rec.dek_wrapped.clone(),
                key_backend: rec.key_backend.clone(),
                key_version: rec.key_version,
                confirmed_at: None,
                last_used_step: None,
            },
        );
        Ok(())
    }
    async fn confirm(&mut self, user: UserId, now: DateTime<Utc>) -> AppResult<()> {
        if let Some(t) = lock(&self.state.totp).get_mut(&user.0) {
            if t.confirmed_at.is_none() {
                t.confirmed_at = Some(now);
            }
        }
        Ok(())
    }
    async fn set_last_used_step(&mut self, user: UserId, step: i64) -> AppResult<()> {
        if let Some(t) = lock(&self.state.totp).get_mut(&user.0) {
            t.last_used_step = Some(step);
        }
        Ok(())
    }
    async fn delete_all(&mut self, user: UserId) -> AppResult<()> {
        lock(&self.state.totp).remove(&user.0);
        lock(&self.state.recovery).retain(|(_, u, _, _)| *u != user.0);
        lock(&self.state.grace).remove(&user.0);
        Ok(())
    }
    async fn replace_recovery_codes(
        &mut self,
        user: UserId,
        _org: OrgId,
        hashes: &[Vec<u8>],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut r = lock(&self.state.recovery);
        r.retain(|(_, u, _, _)| *u != user.0);
        for h in hashes {
            r.push((Uuid::new_v4(), user.0, h.clone(), None));
        }
        Ok(())
    }
    async fn unused_recovery_codes(&mut self, user: UserId) -> AppResult<Vec<RecoveryCodeRow>> {
        Ok(lock(&self.state.recovery)
            .iter()
            .filter(|(_, u, _, used)| *u == user.0 && used.is_none())
            .map(|(id, _, hash, _)| RecoveryCodeRow {
                id: *id,
                code_hash: hash.clone(),
            })
            .collect())
    }
    async fn consume_recovery_code(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<()> {
        if let Some(row) = lock(&self.state.recovery)
            .iter_mut()
            .find(|(i, _, _, _)| *i == id)
        {
            if row.3.is_none() {
                row.3 = Some(now);
            }
        }
        Ok(())
    }
    async fn grace_get(&mut self, user: UserId) -> AppResult<Option<GraceRow>> {
        Ok(lock(&self.state.grace).get(&user.0).cloned())
    }
    async fn grace_bump(
        &mut self,
        user: UserId,
        _org: OrgId,
        now: DateTime<Utc>,
    ) -> AppResult<GraceRow> {
        let mut g = lock(&self.state.grace);
        let entry = g.entry(user.0).or_insert(GraceRow {
            started_at: now,
            logins_since: 0,
        });
        entry.logins_since += 1;
        Ok(entry.clone())
    }
    async fn grace_clear(&mut self, user: UserId) -> AppResult<()> {
        lock(&self.state.grace).remove(&user.0);
        Ok(())
    }
    async fn owners_past_deadline(
        &mut self,
        _now: DateTime<Utc>,
        _max_age_days: i64,
        _max_logins: i32,
    ) -> AppResult<Vec<UserId>> {
        // Der Fake kennt die Rollen nicht; Enforcement-Job wird gegen die DB getestet.
        Ok(Vec::new())
    }
}

/// Test-KeyProvider: wrappt den DEK per XOR mit einem festen Schlüssel.
pub struct FakeTotpKeys;

#[async_trait]
impl TotpKeyProvider for FakeTotpKeys {
    fn backend(&self) -> &'static str {
        "app"
    }
    fn key_version(&self) -> i32 {
        1
    }
    async fn wrap_dek(&self, dek: &[u8]) -> AppResult<Vec<u8>> {
        Ok(dek.iter().map(|b| b ^ 0x5a).collect())
    }
    async fn unwrap_dek(&self, wrapped: &[u8]) -> AppResult<Vec<u8>> {
        Ok(wrapped.iter().map(|b| b ^ 0x5a).collect())
    }
}

#[async_trait]
impl OutboxRepo for FakeTx {
    async fn push(&mut self, event: NewEvent) -> AppResult<EventId> {
        lock(&self.state.events).push(event);
        Ok(EventId::new_v7())
    }
    async fn claim_undelivered(
        &mut self,
        _consumer: &str,
        _limit: i64,
    ) -> AppResult<Vec<StoredEvent>> {
        Ok(vec![])
    }
    async fn mark_delivered(&mut self, _consumer: &str, _ids: &[EventId]) -> AppResult<()> {
        Ok(())
    }
    async fn record_failure(&mut self, _c: &str, _e: EventId, _err: &str) -> AppResult<i32> {
        Ok(1)
    }
    async fn mark_dead_letter(&mut self, _consumer: &str, _event: EventId) -> AppResult<()> {
        Ok(())
    }
    async fn list_dead_letters(
        &mut self,
        _limit: i64,
    ) -> AppResult<Vec<lineal_app::ports::repos::outbox::DeadLetter>> {
        Ok(vec![])
    }
    async fn mark_processed(&mut self, _consumers: &[String]) -> AppResult<u64> {
        Ok(0)
    }
}

#[async_trait]
impl JobRepo for FakeTx {
    async fn enqueue(&mut self, job: NewJob) -> AppResult<Option<JobId>> {
        lock(&self.state.jobs).push(job);
        Ok(Some(JobId::new_v7()))
    }
    async fn claim(
        &mut self,
        _worker: &str,
        _limit: i64,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<Job>> {
        Ok(vec![])
    }
    async fn complete(&mut self, _id: JobId, _now: DateTime<Utc>) -> AppResult<()> {
        Ok(())
    }
    async fn fail(
        &mut self,
        _id: JobId,
        _error: &str,
        _retry_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[async_trait]
impl Tx for FakeTx {
    fn org_id(&self) -> OrgId {
        self.org
    }
    async fn last_sync_id(&mut self) -> AppResult<i64> {
        Ok(self.state.sync_seq.fetch_add(1, Ordering::SeqCst) + 1)
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
    fn totp(&mut self) -> &mut dyn TotpRepo {
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
        Ok(())
    }
}

/// Feste Uhr (Default: 2026-10-10T07:15:00Z).
pub struct FixedClock(pub DateTime<Utc>);

impl Default for FixedClock {
    fn default() -> Self {
        Self(
            Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0)
                .single()
                .unwrap_or_default(),
        )
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Deterministische IDs (fortlaufend, UUID-förmig).
#[derive(Default)]
pub struct SeqIdGen(AtomicU64);

impl IdGen for SeqIdGen {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.0.fetch_add(1, Ordering::SeqCst) + 1))
    }
}

struct FakeBlobs;

#[async_trait]
impl BlobStore for FakeBlobs {
    async fn signed_put_url(&self, key: &str, _ct: &str, _ttl: Duration) -> AppResult<String> {
        Ok(format!("http://fake.invalid/uploads/{key}"))
    }
    async fn signed_get_url(&self, key: &str, _ttl: Duration) -> AppResult<String> {
        Ok(format!("http://fake.invalid/files/{key}"))
    }
}

struct FakeSearch;

#[async_trait]
impl SearchIndex for FakeSearch {
    async fn search(&self, _org: OrgId, _query: &str, _limit: u32) -> AppResult<Vec<SearchHit>> {
        Ok(vec![])
    }
}

struct FakeHasher;

#[async_trait]
impl PasswordHasher for FakeHasher {
    async fn hash(&self, password: &str) -> AppResult<String> {
        Ok(format!("fake:{password}"))
    }
    async fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        Ok(hash == format!("fake:{password}"))
    }
}

struct FakeMailer;

#[async_trait]
impl Mailer for FakeMailer {
    async fn send(&self, _mail: Mail) -> AppResult<()> {
        Ok(())
    }
}

/// Baut eine `App` mit beliebigem Store und Fake-Diensten (feste Uhr, Seq-IDs).
pub fn test_app(store: Arc<dyn Store>) -> Arc<App> {
    Arc::new(App {
        store,
        clock: Arc::new(FixedClock::default()),
        ids: Arc::new(SeqIdGen::default()),
        blobs: Arc::new(FakeBlobs),
        search: Arc::new(FakeSearch),
        hasher: Arc::new(FakeHasher),
        mailer: Arc::new(FakeMailer),
        totp_keys: Arc::new(FakeTotpKeys),
        config: AppConfig::default(),
    })
}

/// `App` komplett ohne Datenbank (`FakeStore`).
pub fn fake_app() -> Arc<App> {
    test_app(Arc::new(FakeStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::{Actor, Scopes};
    use lineal_domain::ids::UserId;

    #[tokio::test]
    async fn configurable_principal_lookup() {
        let principal = Principal {
            kind: lineal_app::ports::repos::CredentialKind::ApiKey,
            org_id: OrgId::new_v7(),
            actor: Actor::App {
                id: UserId::new_v7(),
            },
            scopes: Scopes::read_only(),
            team_scope: None,
        };
        let store = FakeStore::new().with_principal("h1", principal.clone());
        let mut tx = store.begin_system().await.unwrap();
        assert_eq!(
            tx.auth()
                .find_principal_by_token_hash("h1", Utc::now())
                .await
                .unwrap(),
            Some(principal)
        );
        assert_eq!(
            tx.auth()
                .find_principal_by_token_hash("nope", Utc::now())
                .await
                .unwrap(),
            None
        );
    }
}
