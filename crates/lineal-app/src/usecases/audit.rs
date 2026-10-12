//! usecases – Audit-Log (WP25, docs/15 §4.7): Einträge schreiben, lesen und aufbewahren.
//!
//! Schreiben: jede protokollierte Änderung ruft [`App::record_audit`] in **derselben**
//! Transaktion auf (kein Umweg über die Outbox, damit nichts verloren geht). Die Einträge
//! enthalten nie Geheimnisse: keine Tokens, Passwörter, API-Key-Klartexte, Webhook-Secrets und
//! keine Inhalte von Issues.

use crate::app::App;
use crate::context::{Actor, RequestCtx, Scopes};
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::audit::{
    AuditActorKind, AuditCursor, AuditEntry, AuditFilter, NewAuditEntry,
};
use chrono::{DateTime, TimeZone, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, UserId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::IpAddr;
use uuid::Uuid;

/// Standardgröße einer Seite.
pub const AUDIT_PAGE_DEFAULT: i64 = 50;
/// Höchstgröße einer Seite.
pub const AUDIT_PAGE_MAX: i64 = 100;
/// Mindestwert der Aufbewahrung in Tagen.
pub const AUDIT_RETENTION_MIN_DAYS: i64 = 30;
/// Standardwert der Aufbewahrung in Tagen.
pub const AUDIT_RETENTION_DEFAULT_DAYS: i64 = 365;

/// Ein protokolliertes Ereignis. Akteur, IP und Request-ID kommen aus dem Kontext.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEvent {
    pub action: &'static str,
    pub target_type: &'static str,
    pub target_id: Option<Uuid>,
    pub details: Value,
}

impl AuditEvent {
    pub fn new(
        action: &'static str,
        target_type: &'static str,
        target_id: Option<Uuid>,
        details: Value,
    ) -> Self {
        Self {
            action,
            target_type,
            target_id,
            details,
        }
    }
}

/// Kontext für Vorgänge ohne Request-Kontext (Login, Einladungsannahme): handelnd ist ein User.
pub fn user_ctx(
    org_id: OrgId,
    user_id: UserId,
    role: UserRole,
    client_ip: Option<String>,
    request_id: String,
    credential_kind: Option<&'static str>,
) -> RequestCtx {
    RequestCtx {
        org_id,
        actor: Actor::User { id: user_id, role },
        request_id,
        scopes: Scopes::read_only(),
        team_scope: None,
        via_session: credential_kind == Some("session"),
        client_ip,
        credential_kind,
    }
}

/// Baut den Eintrag aus Kontext und Ereignis.
pub fn entry_from_ctx(
    id: Uuid,
    now: DateTime<Utc>,
    ctx: &RequestCtx,
    ev: AuditEvent,
) -> NewAuditEntry {
    let (actor_user_id, actor_kind) = match ctx.actor {
        Actor::User { id, .. } => (Some(id), AuditActorKind::User),
        Actor::App { id } => (Some(id), AuditActorKind::App),
        Actor::System => (None, AuditActorKind::System),
    };
    NewAuditEntry {
        id,
        organization_id: ctx.org_id,
        occurred_at: now,
        actor_user_id,
        actor_kind,
        credential_kind: ctx.credential_kind,
        action: ev.action.to_owned(),
        target_type: ev.target_type.to_owned(),
        target_id: ev.target_id,
        details: ev.details,
        ip: ctx
            .client_ip
            .as_deref()
            .and_then(|s| s.parse::<IpAddr>().ok()),
        request_id: ctx.request_id.chars().take(200).collect(),
    }
}

/// Eingabe der Liste (Filter und Paginierung; bereits aus GraphQL-Argumenten gebaut).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListAuditInput {
    pub first: Option<i64>,
    pub after: Option<AuditCursor>,
    pub action: Option<String>,
    pub actor_user_id: Option<UserId>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

/// Eine Seite des Audit-Logs.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    pub has_next_page: bool,
    /// Cursor des letzten gelieferten Eintrags (für `after`).
    pub end_cursor: Option<AuditCursor>,
    /// Anzeigenamen der Akteure dieser Seite (User-ID -> Name; keine E-Mail-Adressen).
    pub actor_names: BTreeMap<Uuid, String>,
}

impl AuditCursor {
    /// Opaker Text für Clients: `<Mikrosekunden>_<uuid>`.
    pub fn encode(&self) -> String {
        format!("{}_{}", self.occurred_at.timestamp_micros(), self.id)
    }

    /// Gegenstück zu [`AuditCursor::encode`]; `None` bei ungültigem Text.
    pub fn decode(text: &str) -> Option<Self> {
        let (micros, id) = text.split_once('_')?;
        let micros: i64 = micros.parse().ok()?;
        let occurred_at = Utc.timestamp_micros(micros).single()?;
        let id = Uuid::parse_str(id).ok()?;
        Some(Self { occurred_at, id })
    }
}

impl App {
    /// Schreibt einen Audit-Eintrag in die übergebene Transaktion.
    pub async fn record_audit(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        event: AuditEvent,
    ) -> AppResult<()> {
        let entry = entry_from_ctx(self.ids.new_uuid(), self.clock.now(), ctx, event);
        tx.audit().append(&entry).await
    }

    /// Liste des Audit-Logs (nur Owner/Admin mit Scope `read` und `admin`).
    pub async fn list_audit_log(
        &self,
        ctx: &RequestCtx,
        input: ListAuditInput,
    ) -> AppResult<AuditPage> {
        if !matches!(ctx.actor, Actor::User { .. }) {
            return Err(AppError::forbidden(
                "The audit log is only available to users.",
            ));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        // Eigene Prüfung statt `require_workspace`: Lesen braucht keinen Schreib-Scope.
        // Team-beschränkte Keys sehen nur ihre Teams; das Audit-Log ist workspaceweit.
        if !snap.actor.is_admin()
            || !ctx.scopes.read
            || !ctx.scopes.admin
            || ctx.team_scope.is_some()
        {
            return Err(AppError::forbidden(
                "The audit log is only available to workspace owners and admins.",
            ));
        }
        let limit = input.first.unwrap_or(AUDIT_PAGE_DEFAULT);
        if !(1..=AUDIT_PAGE_MAX).contains(&limit) {
            return Err(AppError::invalid_request(format!(
                "first must be between 1 and {AUDIT_PAGE_MAX}"
            ))
            .with_details(json!({ "field": "first" })));
        }
        if let Some(a) = &input.action {
            if a.is_empty() || a.chars().count() > 100 {
                return Err(AppError::invalid_request("action must be 1-100 characters")
                    .with_details(json!({ "field": "action" })));
            }
        }
        if let (Some(s), Some(u)) = (input.since, input.until) {
            if s >= u {
                return Err(AppError::invalid_request("since must be before until")
                    .with_details(json!({ "field": "since" })));
            }
        }
        let filter = AuditFilter {
            action: input.action.clone(),
            actor_user_id: input.actor_user_id,
            since: input.since,
            until: input.until,
        };
        // Eine Zeile mehr laden, um `hasNextPage` zu bestimmen.
        let mut rows = tx.audit().list(&filter, input.after, limit + 1).await?;
        let has_next_page = rows.len() as i64 > limit;
        rows.truncate(usize::try_from(limit).unwrap_or(0));
        let end_cursor = rows.last().map(|r| AuditCursor {
            occurred_at: r.occurred_at,
            id: r.id,
        });
        let mut actor_names = BTreeMap::new();
        for actor in rows.iter().filter_map(|r| r.actor_user_id) {
            if !actor_names.contains_key(&actor.0)
                && let Some(user) = tx.users().get(actor).await?
            {
                actor_names.insert(actor.0, user.name);
            }
        }
        Ok(AuditPage {
            entries: rows,
            has_next_page,
            end_cursor,
            actor_names,
        })
    }

    /// Aufbewahrung: löscht abgelaufene Einträge aller Workspaces (Worker, System).
    pub async fn purge_audit_log(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let mut tx = self.store.begin_system().await?;
        let n = tx.audit().purge_expired(now).await?;
        tx.commit().await?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let c = AuditCursor {
            occurred_at: Utc
                .with_ymd_and_hms(2026, 10, 10, 7, 15, 0)
                .single()
                .unwrap_or_default(),
            id: Uuid::nil(),
        };
        assert_eq!(AuditCursor::decode(&c.encode()), Some(c));
        assert_eq!(AuditCursor::decode("garbage"), None);
        assert_eq!(AuditCursor::decode("12_nicht-eine-uuid"), None);
    }
}
