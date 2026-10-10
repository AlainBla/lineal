//! Repository-Port `OutboxRepo` (Transactional Outbox, docs/09 §6 und docs/11 §3.6).

use crate::error::AppResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{EventId, OrgId, UserId};

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

    /// Setzt `processed_at` bei allen Events, die an alle `consumers` zugestellt wurden.
    async fn mark_processed(&mut self, consumers: &[String]) -> AppResult<u64>;
}
