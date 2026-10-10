//! Tool-Handler der Gruppe `views` (WP11): `list_custom_views` (docs/07 §3.x). Nur Metadaten;
//! die Filterdefinition wird nie ausgegeben. Archivierte sowie an Projekte/Initiativen gebundene
//! Views fehlen; unsichtbare Views existieren nicht (docs/05 §4.2 Regel 5).

use super::teams_users::ts;
use crate::envelope::{decode_cursor, encode_cursor, page, parse_limit, query_fingerprint};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::user::{OrderBy, PageCursor, PageRequest};
use lineal_app::ports::repos::view::ViewModel;
use lineal_app::usecases::view::{ListViewsInput, ViewInfo};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn str_arg<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(AppError::invalid_request(format!("`{key}` must be a string"))
            .with_details(json!({ "path": format!("/{key}") }))),
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 50 {
            break;
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn view_json(v: &ViewInfo, base: &str) -> Value {
    let c = &v.view;
    let slug = slugify(&c.name);
    let url = if slug.is_empty() {
        format!("{base}/view/{}", c.slug_id)
    } else {
        format!("{base}/view/{slug}-{}", c.slug_id)
    };
    json!({
        "id": c.id.to_string(),
        "name": c.name,
        "description": c.description,
        "modelName": c.model.as_str(),
        "shared": c.shared,
        "team": v.team.as_ref().map(|(key, name)| json!({
            "id": c.team_id.map(|t| t.to_string()), "key": key, "name": name,
        })),
        "owner": json!({ "id": c.owner_id.to_string(), "name": v.owner_name }),
        "url": url,
        "createdAt": ts(c.created_at),
        "updatedAt": ts(c.updated_at),
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_custom_views" => {
            let limit = parse_limit(&args, 50, 250)?;
            let fingerprint = query_fingerprint(&args);
            let order_by = match str_arg(&args, "orderBy")? {
                Some("createdAt") => OrderBy::CreatedAt,
                _ => OrderBy::UpdatedAt,
            };
            let cursor = match str_arg(&args, "cursor")? {
                Some(c) => {
                    let pos = decode_cursor(c, &fingerprint)?;
                    let bad = || AppError::invalid_request("Invalid cursor");
                    let key = pos
                        .key
                        .as_str()
                        .and_then(|k| DateTime::parse_from_rfc3339(k).ok())
                        .ok_or_else(bad)?
                        .with_timezone(&Utc);
                    let id = pos.id.parse().map_err(|_| bad())?;
                    Some(PageCursor { key, id })
                }
                None => None,
            };
            let model = match str_arg(&args, "type")? {
                Some(t) => Some(ViewModel::parse(t).ok_or_else(|| {
                    AppError::invalid_request("`type` must be Issue, Project or Initiative")
                })?),
                None => None,
            };
            let res = app
                .list_views(
                    ctx,
                    ListViewsInput {
                        model,
                        team: str_arg(&args, "team")?.map(str::to_owned),
                        shared: args.get("shared").and_then(Value::as_bool),
                        query: str_arg(&args, "query")?.map(str::to_owned),
                        page: PageRequest {
                            limit: limit as i64,
                            cursor,
                            order_by,
                        },
                    },
                )
                .await?;
            let org = app.get_workspace(ctx).await?;
            let base = app.workspace_url(&org);
            let items = res.items.iter().map(|v| view_json(v, &base)).collect();
            let next = res.next.map(|c| {
                encode_cursor(
                    &json!(c.key.to_rfc3339_opts(SecondsFormat::Micros, true)),
                    c.id,
                    &fingerprint,
                )
            });
            Ok(page("customViews", items, next))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by views"
        ))),
    }
}
