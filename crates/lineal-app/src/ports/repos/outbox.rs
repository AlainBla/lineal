//! Repository-Port `OutboxRepo` (Transactional Outbox, docs/09 §6 und docs/11 §3.6).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{EventId, OrgId, UserId};
use serde_json::Value;
use uuid::Uuid;

/// Neues Event, das in der laufenden Transaktion geschrieben wird.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub event: DomainEvent,
    pub origin: EventOrigin,
    /// Auslösender User (für Self-Suppression bei Notifications); `None` bei System/App.
    pub actor_id: Option<UserId>,
}

impl NewEvent {
    pub fn new(event: DomainEvent, origin: EventOrigin) -> Self {
        Self {
            event,
            origin,
            actor_id: None,
        }
    }

    /// Event eines Requests: Origin `user`, Auslöser aus `ctx.actor` (nur bei User-Akteuren).
    pub fn from_ctx(event: DomainEvent, ctx: &crate::RequestCtx) -> Self {
        let actor_id = match ctx.actor {
            crate::Actor::User { id, .. } | crate::Actor::App { id } => Some(id),
            crate::Actor::System => None,
        };
        let origin = match ctx.actor {
            crate::Actor::System => EventOrigin::System,
            _ => EventOrigin::User,
        };
        Self {
            event,
            origin,
            actor_id,
        }
    }

    pub fn with_actor(mut self, actor_id: Option<UserId>) -> Self {
        self.actor_id = actor_id;
        self
    }
}

/// Gespeichertes Event.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub id: EventId,
    pub org_id: OrgId,
    pub origin: EventOrigin,
    pub actor_id: Option<UserId>,
    pub sync_id: i64,
    pub event: DomainEvent,
    pub created_at: DateTime<Utc>,
}

/// Ein aufgegebenes Event eines Konsumenten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub consumer: String,
    pub event_id: EventId,
    pub attempts: i32,
    pub last_error: String,
}

#[async_trait]
pub trait OutboxRepo: Send {
    /// Schreibt ein Event mit der `lastSyncId` der Transaktion (selber Commit wie die Änderung).
    async fn push(&mut self, event: NewEvent) -> AppResult<EventId>;

    /// Holt bis zu `limit` Events, die `consumer` noch nicht verarbeitet hat
    /// (`ORDER BY sync_id … FOR UPDATE SKIP LOCKED`, mandantenübergreifend; nur mit `begin_system`).
    async fn claim_undelivered(
        &mut self,
        consumer: &str,
        limit: i64,
    ) -> AppResult<Vec<StoredEvent>>;

    /// Vermerkt die Zustellung an `consumer` (`outbox_delivery`), idempotent.
    async fn mark_delivered(&mut self, consumer: &str, ids: &[EventId]) -> AppResult<()>;

    /// Zählt einen Fehlversuch von `consumer` für das Event dauerhaft (Zeile ohne `delivered_at`)
    /// und merkt sich die Fehlermeldung. Liefert die Zahl der Versuche inkl. dieses.
    async fn record_failure(
        &mut self,
        consumer: &str,
        event: EventId,
        error: &str,
    ) -> AppResult<i32>;

    /// Gibt das Event für `consumer` auf (Dead-Letter): gilt als zugestellt, `last_error` bleibt.
    async fn mark_dead_letter(&mut self, consumer: &str, event: EventId) -> AppResult<()>;

    /// Aufgegebene Events (Dead-Letter), neueste zuerst.
    async fn list_dead_letters(&mut self, limit: i64) -> AppResult<Vec<DeadLetter>>;

    /// Setzt `processed_at` bei allen Events, die an alle `consumers` zugestellt wurden.
    async fn mark_processed(&mut self, consumers: &[String]) -> AppResult<u64>;

    // --- Sync-Log (WP15, docs/09 §5). Alle Methoden sind auf `Tx::org_id()` begrenzt. ---

    /// Schreibt Aktionen ins `sync_log` (idempotent über `(event_id, n)`), vergibt die
    /// Stream-IDs in Commit-Reihenfolge und weckt per `NOTIFY` die Verbindungen. Liefert die
    /// höchste vergebene ID (`None`, wenn alles schon vorhanden war).
    async fn sync_append(&mut self, _actions: &[NewSyncAction]) -> AppResult<Option<i64>> {
        sync_unimplemented()
    }

    /// Aktueller Stand des Logs dieses Workspace.
    async fn sync_head(&mut self) -> AppResult<SyncHead> {
        sync_unimplemented()
    }

    /// Aktionen mit `after < id <= upto`, deren Gruppen sich mit `groups` überschneiden, aufsteigend.
    async fn sync_read(
        &mut self,
        _after: i64,
        _upto: i64,
        _groups: &[String],
        _limit: i64,
    ) -> AppResult<Vec<SyncActionRow>> {
        sync_unimplemented()
    }

    /// Gruppen der letzten (nicht gelöschten) Aktion zu diesem Objekt.
    async fn sync_last_groups(
        &mut self,
        _model_name: &str,
        _model_id: Uuid,
    ) -> AppResult<Option<Vec<String>>> {
        sync_unimplemented()
    }

    /// Löscht Einträge, die vor `before` geschrieben wurden, und setzt das Wasserzeichen.
    async fn sync_purge(&mut self, _before: DateTime<Utc>) -> AppResult<u64> {
        sync_unimplemented()
    }

    /// Aktueller Zustand eines Objekts als Sync-`data` plus seine Sync-Gruppen (zum Zeitpunkt des
    /// Aufrufs). `None`, wenn es nicht (mehr) existiert oder `model_name` unbekannt ist.
    async fn sync_model(
        &mut self,
        _model_name: &str,
        _model_id: Uuid,
    ) -> AppResult<Option<SyncModelState>> {
        sync_unimplemented()
    }
}

fn sync_unimplemented<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Neue Aktion für das Sync-Log.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSyncAction {
    pub event_id: EventId,
    /// Laufende Nummer innerhalb des Events (mehrere Aktionen je Event möglich).
    pub n: i16,
    /// `lastSyncId` der auslösenden Transaktion.
    pub sync_id: i64,
    /// `I`, `U`, `A`, `V`, `D`, `C` oder `G`.
    pub action: String,
    pub model_name: String,
    pub model_id: Uuid,
    pub data: Value,
    /// `team:<uuid>`, `org`, `user:<uuid>`.
    pub groups: Vec<String>,
    pub origin: EventOrigin,
    pub actor_id: Option<UserId>,
}

/// Zeile des Sync-Logs.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncActionRow {
    /// Stream-ID (`lastSyncId` des Clients).
    pub id: i64,
    pub sync_id: i64,
    pub action: String,
    pub model_name: String,
    pub model_id: Uuid,
    pub data: Value,
    pub groups: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Stand des Sync-Logs eines Workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncHead {
    /// Höchste vergebene ID (0, wenn leer).
    pub head: i64,
    /// Alles bis einschließlich dieser ID wurde durch die Aufbewahrung gelöscht (0 = nichts).
    pub purged_up_to: i64,
}

/// Sync-Zustand eines Objekts.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncModelState {
    pub data: Value,
    pub groups: Vec<String>,
}
