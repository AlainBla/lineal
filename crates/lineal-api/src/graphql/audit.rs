//! GraphQL: Audit-Log (`auditLog`, WP25). Nur Owner/Admin mit Scope `read` und `admin`; alles
//! andere `forbidden`. Cursor-Pagination, höchstens 100 Einträge je Seite.
//!
//! `details` ist der gespeicherte JSON-Text (nie Geheimnisse, siehe lineal-app usecases/audit).
//! E-Mail-Adressen erscheinen nicht; Akteure werden über Name und ID dargestellt.

use crate::context::{app, request_ctx};
use crate::error::GqlResultExt;
use async_graphql::{Context, Object, SimpleObject};
use chrono::{DateTime, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::{AuditCursor, AuditEntry};
use lineal_app::usecases::audit::{AUDIT_PAGE_DEFAULT, ListAuditInput};
use lineal_domain::ids::UserId;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Eintrag des Audit-Logs.
#[derive(SimpleObject)]
#[graphql(name = "AuditLogEntry")]
pub struct AuditLogEntryGql {
    pub id: String,
    pub occurred_at: String,
    /// `user`, `app` oder `system`.
    pub actor_kind: String,
    pub actor_id: Option<String>,
    /// Anzeigename des Akteurs (nur, solange der Akteur existiert).
    pub actor_name: Option<String>,
    /// `session`, `api_key`, `oauth`, `cli` oder `null`.
    pub credential_kind: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    /// Details als JSON-Text (ohne Geheimnisse).
    pub details: String,
    pub ip: Option<String>,
    pub request_id: String,
}

#[derive(SimpleObject)]
#[graphql(name = "AuditLogPageInfo")]
pub struct AuditLogPageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "AuditLogConnection")]
pub struct AuditLogConnection {
    pub nodes: Vec<AuditLogEntryGql>,
    pub page_info: AuditLogPageInfo,
}

fn entry_to_gql(e: AuditEntry, names: &BTreeMap<Uuid, String>) -> AuditLogEntryGql {
    let actor_name = e.actor_user_id.and_then(|u| names.get(&u.0).cloned());
    AuditLogEntryGql {
        id: e.id.to_string(),
        occurred_at: e.occurred_at.to_rfc3339(),
        actor_kind: e.actor_kind,
        actor_id: e.actor_user_id.map(|u| u.0.to_string()),
        actor_name,
        credential_kind: e.credential_kind,
        action: e.action,
        target_type: e.target_type,
        target_id: e.target_id.map(|t| t.to_string()),
        details: e.details.to_string(),
        ip: e.ip,
        request_id: e.request_id,
    }
}

fn invalid(field: &str, message: &str) -> async_graphql::Error {
    crate::graphql::org::gql_err(
        AppError::invalid_request(message).with_details(serde_json::json!({ "field": field })),
    )
}

fn parse_time(field: &str, raw: &str) -> async_graphql::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| invalid(field, "must be an ISO-8601 timestamp"))
}

#[derive(Default)]
pub struct AuditQuery;

#[Object]
impl AuditQuery {
    /// Audit-Log des Workspaces, neueste zuerst. Nur Owner/Admin mit Scope `admin`.
    #[allow(clippy::too_many_arguments)]
    async fn audit_log(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        action: Option<String>,
        actor_id: Option<String>,
        since: Option<String>,
        until: Option<String>,
    ) -> async_graphql::Result<AuditLogConnection> {
        let first = first.map_or(AUDIT_PAGE_DEFAULT, i64::from);
        let after = match after {
            Some(c) => {
                Some(AuditCursor::decode(&c).ok_or_else(|| invalid("after", "invalid cursor"))?)
            }
            None => None,
        };
        let actor_user_id = match actor_id {
            Some(a) => Some(UserId(
                Uuid::parse_str(&a).map_err(|_| invalid("actorId", "invalid id"))?,
            )),
            None => None,
        };
        let input = ListAuditInput {
            first: Some(first),
            after,
            action,
            actor_user_id,
            since: since
                .as_deref()
                .map(|s| parse_time("since", s))
                .transpose()?,
            until: until
                .as_deref()
                .map(|s| parse_time("until", s))
                .transpose()?,
        };
        let page = app(ctx)?
            .list_audit_log(request_ctx(ctx)?, input)
            .await
            .gql(ctx)?;
        let end_cursor = page.end_cursor.map(|c| c.encode());
        let has_next_page = page.has_next_page;
        let names = page.actor_names;
        let nodes = page
            .entries
            .into_iter()
            .map(|e| entry_to_gql(e, &names))
            .collect();
        Ok(AuditLogConnection {
            nodes,
            page_info: AuditLogPageInfo {
                has_next_page,
                end_cursor,
            },
        })
    }
}
