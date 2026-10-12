//! Repository-Port `AuditRepo`: append-only Audit-Log je Workspace (docs/15 §4.7, WP25).
//!
//! Einträge werden in derselben Transaktion wie die protokollierte Änderung geschrieben (nicht
//! über die Outbox). Gelöscht wird nur über die Aufbewahrung (`purge_expired`).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::{OrgId, UserId};
use serde_json::Value;
use std::net::IpAddr;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Art des Handelnden (Spalte `actor_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditActorKind {
    User,
    App,
    System,
}

impl AuditActorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::App => "app",
            Self::System => "system",
        }
    }
}

/// Neuer Eintrag. Enthält nie Geheimnisse (Tokens, Passwörter, Key-Klartext, Webhook-Secrets).
#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEntry {
    pub id: Uuid,
    pub organization_id: OrgId,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<UserId>,
    pub actor_kind: AuditActorKind,
    /// `session` | `api_key` | `oauth` | `cli`; `None` ohne Zugangsdaten.
    pub credential_kind: Option<&'static str>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    /// Objekt (Schlüssel/Werte ohne Geheimnisse).
    pub details: Value,
    pub ip: Option<IpAddr>,
    pub request_id: String,
}

/// Gespeicherter Eintrag.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub id: Uuid,
    pub organization_id: OrgId,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<UserId>,
    pub actor_kind: String,
    pub credential_kind: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub details: Value,
    pub ip: Option<String>,
    pub request_id: String,
}

/// Filter der Liste (alle Felder optional, UND-verknüpft).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub actor_user_id: Option<UserId>,
    /// Inklusive.
    pub since: Option<DateTime<Utc>>,
    /// Exklusive.
    pub until: Option<DateTime<Utc>>,
}

/// Keyset-Cursor: letzter gelieferter Eintrag (Sortierung `occurred_at desc, id desc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor {
    pub occurred_at: DateTime<Utc>,
    pub id: Uuid,
}

#[async_trait]
pub trait AuditRepo: Send {
    /// Hängt einen Eintrag an. Die `organization_id` des Eintrags wird verwendet (Systemkontexte
    /// schreiben explizit für den betroffenen Workspace).
    async fn append(&mut self, _entry: &NewAuditEntry) -> AppResult<()> {
        unimplemented_repo()
    }

    /// Liefert bis zu `limit` Einträge des Workspaces der Transaktion, neueste zuerst, nach dem
    /// Cursor (exklusiv). Mandantenfilter ist immer `organization_id` der Transaktion.
    async fn list(
        &mut self,
        _filter: &AuditFilter,
        _cursor: Option<AuditCursor>,
        _limit: i64,
    ) -> AppResult<Vec<AuditEntry>> {
        unimplemented_repo()
    }

    /// Aufbewahrung: löscht Einträge älter als `auditLogRetentionDays` je Workspace (Default 365,
    /// Minimum 30). Liefert die Anzahl gelöschter Zeilen. Nur aus dem Worker aufrufen.
    async fn purge_expired(&mut self, _now: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }
}
