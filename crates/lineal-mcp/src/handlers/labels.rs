//! Tool-Handler der Gruppe `labels` (WP06): Issue-, Projekt- und Initiative-Labels
//! (`list_*_labels`, `save_*_label`, `retire_*_label`, `restore_*_label`, docs/07 §3.4).

use super::teams_users::ts as ts_millis;
use crate::envelope::{
    decode_cursor, encode_cursor, page, parse_limit, query_fingerprint, team_ref,
};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::label::{LabelView, ListLabelsInput, SaveLabelInput};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::enums::LabelGroupType;
use serde_json::{Value, json};

fn bad(path: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "path": format!("/{path}") }))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(bad(key, format!("`{key}` must be a string"))),
    }
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn label_json(v: &LabelView, with_affected: bool) -> Value {
    let l = &v.label;
    let group_type = if l.is_group {
        json!(l.effective_group_type().as_str())
    } else {
        Value::Null
    };
    let mut j = json!({
        "id": l.id.to_string(),
        "name": l.name,
        "color": l.color,
        "description": l.description,
        "isGroup": l.is_group,
        "groupType": group_type,
        "parent": v.parent.as_ref().map(|(id, name)| json!({ "id": id.to_string(), "name": name })),
        "team": v.team.as_ref().map(|(id, key, name)| team_ref(id, key, name)),
        "retiredAt": l.retired_at.map(ts_millis),
        "archivedAt": l.archived_at.map(ts_millis),
        "createdAt": ts_millis(l.created_at),
        "updatedAt": ts_millis(l.updated_at),
    });
    if with_affected
        && l.is_group
        && let Some(o) = j.as_object_mut()
    {
        o.insert(
            "affectedChildren".into(),
            json!(
                v.affected_children
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
            ),
        );
    }
    j
}

fn kind_of(tool: &str) -> Option<LabelKind> {
    if tool.ends_with("issue_label") || tool == "list_issue_labels" {
        Some(LabelKind::Issue)
    } else if tool.ends_with("project_label") || tool == "list_project_labels" {
        Some(LabelKind::Project)
    } else if tool.ends_with("initiative_label") || tool == "list_initiative_labels" {
        Some(LabelKind::Initiative)
    } else {
        None
    }
}

fn tri(args: &Value, key: &str) -> AppResult<Option<Option<String>>> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(bad(key, format!("`{key}` must be a string or null"))),
    }
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    let kind = kind_of(tool)
        .ok_or_else(|| AppError::internal(format!("tool `{tool}` is not handled by labels")))?;
    if tool.starts_with("list_") {
        let limit = parse_limit(&args, 50, 250)?;
        let fingerprint = query_fingerprint(&args);
        let cursor = match arg_str(&args, "cursor")? {
            Some(c) => {
                let pos = decode_cursor(c, &fingerprint)?;
                let key = pos
                    .key
                    .as_str()
                    .and_then(|k| DateTime::parse_from_rfc3339(k).ok())
                    .ok_or_else(|| AppError::invalid_request("Invalid cursor"))?
                    .with_timezone(&Utc);
                let id = pos
                    .id
                    .parse()
                    .map_err(|_| AppError::invalid_request("Invalid cursor"))?;
                Some(PageCursor { key, id })
            }
            None => None,
        };
        let res = app
            .list_labels(
                ctx,
                kind,
                ListLabelsInput {
                    team: arg_str(&args, "team")?.map(str::to_owned),
                    name: arg_str(&args, "name")?.map(str::to_owned),
                    include_groups: flag(&args, "includeGroups"),
                    include_archived: flag(&args, "includeArchived"),
                    order_by: match arg_str(&args, "orderBy")? {
                        Some("createdAt") => OrderBy::CreatedAt,
                        _ => OrderBy::UpdatedAt,
                    },
                    limit: limit as i64,
                    cursor,
                },
            )
            .await?;
        let next = res.next.map(|c| {
            encode_cursor(
                &json!(c.key.to_rfc3339_opts(SecondsFormat::Micros, true)),
                c.id,
                &fingerprint,
            )
        });
        let items = res.items.iter().map(|v| label_json(v, false)).collect();
        return Ok(page("labels", items, next));
    }
    if tool.starts_with("save_") {
        let group_type = match arg_str(&args, "groupType")? {
            None => None,
            Some(s) => Some(s.parse::<LabelGroupType>().map_err(|_| {
                bad(
                    "groupType",
                    "`groupType` must be singleSelect or multiSelect",
                )
            })?),
        };
        let input = SaveLabelInput {
            id: arg_str(&args, "id")?.map(str::to_owned),
            name: arg_str(&args, "name")?.map(str::to_owned),
            color: arg_str(&args, "color")?.map(str::to_owned),
            description: tri(&args, "description")?,
            team_id: arg_str(&args, "teamId")?.map(str::to_owned),
            parent: tri(&args, "parent")?,
            is_group: args.get("isGroup").and_then(Value::as_bool),
            group_type,
        };
        let v = app.save_label(ctx, kind, input).await?;
        return Ok(label_json(&v, false));
    }
    let id = arg_str(&args, "id")?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad("id", "`id` is required"))?;
    let v = if tool.starts_with("retire_") {
        app.retire_label(ctx, kind, id).await?
    } else {
        app.restore_label(ctx, kind, id).await?
    };
    Ok(label_json(&v, true))
}
