//! Tool-Handler der Gruppe `inbox` (WP08): `get_notifications`, `mark_notification`
//! (docs/07 §3.13, docs/05 §6). Die Eingabe wurde bereits gegen das Snapshot-Schema validiert.

use crate::envelope::{
    CursorPos, decode_cursor, encode_cursor, page, parse_limit, project_ref, query_fingerprint,
    user_ref,
};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::user::{OrderBy, PageCursor, PageRequest};
use lineal_app::usecases::notification::{
    ListNotificationsInput, MarkNotificationInput, NotificationItem,
};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::ids::NotificationId;
use serde_json::{Value, json};
use std::str::FromStr;
use uuid::Uuid;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn ts_opt(t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| Value::String(ts(t)))
}

fn notification_json(item: &NotificationItem) -> Value {
    let v = &item.view;
    let n = &v.n;
    let issue = match (&v.issue_identifier, &v.issue_title, n.issue_id) {
        (Some(identifier), Some(title), Some(uuid)) => {
            json!({ "id": identifier, "uuid": uuid.to_string(), "title": title })
        }
        _ => Value::Null,
    };
    let project = match (n.project_id, &v.project_name) {
        (Some(id), Some(name)) => project_ref(id, &id.to_string(), name),
        _ => Value::Null,
    };
    let initiative = match (n.initiative_id, &v.initiative_name) {
        (Some(id), Some(name)) => {
            json!({ "id": id.to_string(), "identifier": id.to_string(), "name": name })
        }
        _ => Value::Null,
    };
    let document = match (n.document_id, &v.document_title) {
        (Some(id), Some(title)) => json!({ "id": id.to_string(), "title": title }),
        _ => Value::Null,
    };
    let actor = match (n.actor_id, &v.actor_name) {
        (Some(id), Some(name)) => user_ref(id, name),
        _ => Value::Null,
    };
    json!({
        "id": n.id.to_string(),
        "type": n.kind,
        "category": n.category,
        "title": item.title,
        "subtitle": item.subtitle,
        "url": item.url,
        "issue": issue,
        "project": project,
        "initiative": initiative,
        "document": document,
        "actor": actor,
        "isPriority": n.is_priority,
        "createdAt": ts(n.created_at),
        "updatedAt": ts(n.updated_at),
        "readAt": ts_opt(n.read_at),
        "archivedAt": ts_opt(n.archived_at),
        "snoozedUntilAt": ts_opt(n.snoozed_until_at),
    })
}

fn cursor_of(item: &NotificationItem, hash: &str) -> String {
    let n = &item.view.n;
    encode_cursor(
        &Value::String(n.created_at.to_rfc3339_opts(SecondsFormat::Micros, true)),
        n.id,
        hash,
    )
}

fn page_cursor(pos: &CursorPos) -> AppResult<PageCursor> {
    let invalid = || AppError::invalid_request("Invalid cursor");
    let key = pos.key.as_str().ok_or_else(invalid)?;
    Ok(PageCursor {
        key: DateTime::parse_from_rfc3339(key)
            .map_err(|_| invalid())?
            .with_timezone(&Utc),
        id: Uuid::parse_str(&pos.id).map_err(|_| invalid())?,
    })
}

fn not_found() -> AppError {
    AppError::not_found("Notification not found.")
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "get_notifications" => {
            let limit = parse_limit(&args, 50, 250)?;
            let hash = query_fingerprint(&args);
            let cursor = match args.get("cursor").and_then(Value::as_str) {
                Some(c) if !c.is_empty() => Some(page_cursor(&decode_cursor(c, &hash)?)?),
                _ => None,
            };
            let unread_only = args
                .get("unreadOnly")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let page_data = app
                .list_notifications(
                    ctx,
                    ListNotificationsInput {
                        unread_only,
                        page: PageRequest {
                            limit: i64::try_from(limit).unwrap_or(50),
                            cursor,
                            order_by: OrderBy::CreatedAt,
                        },
                        ..Default::default()
                    },
                )
                .await?;
            let next_cursor = page_data
                .next
                .and_then(|_| page_data.items.last().map(|i| cursor_of(i, &hash)));
            let items = page_data.items;
            Ok(page(
                "notifications",
                items.iter().map(notification_json).collect(),
                next_cursor,
            ))
        }
        "mark_notification" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .and_then(|s| NotificationId::from_str(s.trim()).ok())
                .ok_or_else(not_found)?;
            let read = args.get("read").and_then(Value::as_bool);
            let snoozed = match args.get("snoozedUntilAt") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(Value::String(s)) => Some(Some(
                    DateTime::parse_from_rfc3339(s)
                        .map_err(|_| {
                            AppError::invalid_request("`snoozedUntilAt` must be an ISO-8601 time.")
                                .with_details(json!({ "path": "/snoozedUntilAt" }))
                        })?
                        .with_timezone(&Utc),
                )),
                Some(_) => {
                    return Err(AppError::invalid_request(
                        "`snoozedUntilAt` must be an ISO-8601 time or null.",
                    )
                    .with_details(json!({ "path": "/snoozedUntilAt" })));
                }
            };
            let item = app
                .mark_notification(
                    ctx,
                    id,
                    MarkNotificationInput {
                        read,
                        snoozed_until_at: snoozed,
                    },
                )
                .await?;
            Ok(notification_json(&item))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by inbox"
        ))),
    }
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
