//! Tool-Handler der Gruppe `status_updates` (WP07): `get_status_updates`, `save_status_update`,
//! `delete_status_update` (docs/07 §3.11). Beide Typen (`project`, `initiative`) teilen eine Form.

use super::initiatives::initiative_ref;
use super::projects::{arg_bool, arg_str, required, url_of, user_ref};
use super::teams_users::{envelope, page_request, parse_time_filter, ts};
use lineal_app::ports::repos::initiative::InitiativeUpdate;
use lineal_app::ports::repos::project::ProjectUpdate;
use lineal_app::usecases::initiative::update::{InitiativeUpdateView, ListInitiativeUpdatesInput};
use lineal_app::usecases::project::update::{ListProjectUpdatesInput, ProjectUpdateView};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::enums::Health;
use serde_json::{Value, json};

#[allow(clippy::too_many_arguments)]
fn common(
    kind: &str,
    id: &str,
    body: &str,
    health: Health,
    url: String,
    created: chrono::DateTime<chrono::Utc>,
    updated: chrono::DateTime<chrono::Utc>,
    edited: Option<chrono::DateTime<chrono::Utc>>,
    archived: Option<chrono::DateTime<chrono::Utc>>,
    diff_md: &Option<String>,
    hidden: bool,
) -> Value {
    json!({
        "id": id, "type": kind, "body": body, "health": health.as_str(), "url": url,
        "createdAt": ts(created), "updatedAt": ts(updated), "editedAt": edited.map(ts), "archivedAt": archived.map(ts),
        "diffMarkdown": if hidden { Value::Null } else { json!(diff_md) }, "isDiffHidden": hidden,
    })
}

fn project_update_json(ws: &str, v: &ProjectUpdateView) -> Value {
    let u: &ProjectUpdate = &v.update;
    let mut j = common(
        "project",
        &u.id.to_string(),
        &u.body,
        u.health,
        format!(
            "{}#project-update-{}",
            url_of(ws, "project", &v.project.name, &v.project_slug) + "/updates",
            &u.id.to_string()[..8]
        ),
        u.created_at,
        u.updated_at,
        u.edited_at,
        u.archived_at,
        &u.diff_markdown,
        u.is_diff_hidden,
    );
    j["user"] = v.user.as_ref().map_or(Value::Null, user_ref);
    j["project"] = json!({ "id": v.project.id.to_string(), "identifier": v.project.identifier, "name": v.project.name });
    j["initiative"] = Value::Null;
    j["isStale"] = json!(v.is_stale);
    j["commentCount"] = json!(v.comment_count);
    j
}

fn initiative_update_json(ws: &str, v: &InitiativeUpdateView) -> Value {
    let u: &InitiativeUpdate = &v.update;
    let mut j = common(
        "initiative",
        &u.id.to_string(),
        &u.body,
        u.health,
        format!(
            "{}#initiative-update-{}",
            url_of(ws, "initiative", &v.initiative.name, &v.initiative.slug_id) + "/updates",
            &u.id.to_string()[..8]
        ),
        u.created_at,
        u.updated_at,
        u.edited_at,
        u.archived_at,
        &u.diff_markdown,
        u.is_diff_hidden,
    );
    j["user"] = v.user.as_ref().map_or(Value::Null, user_ref);
    j["project"] = Value::Null;
    j["initiative"] = initiative_ref(&v.initiative);
    j["isStale"] = json!(v.is_stale);
    j["commentCount"] = json!(v.comment_count);
    j
}

fn health(args: &Value) -> AppResult<Option<Health>> {
    arg_str(args, "health")
        .map(|h| {
            h.parse::<Health>().map_err(|_| {
                AppError::invalid_request("`health` must be onTrack, atRisk or offTrack")
            })
        })
        .transpose()
}

fn wrong_parent(kind: &str) -> AppError {
    let other = if kind == "project" {
        "initiative"
    } else {
        "project"
    };
    AppError::invalid_request(format!("`{other}` cannot be used with type `{kind}`"))
        .with_details(json!({ "path": format!("/{other}") }))
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    let kind = required(&args, "type")?.to_owned();
    if kind != "project" && kind != "initiative" {
        return Err(AppError::invalid_request(
            "`type` must be project or initiative",
        ));
    }
    let ws = {
        let org = app.get_workspace(ctx).await?;
        app.workspace_url(&org)
    };
    match tool {
        "get_status_updates" => {
            if kind == "project" && arg_str(&args, "initiative").is_some() {
                return Err(wrong_parent("project"));
            }
            if kind == "initiative" && arg_str(&args, "project").is_some() {
                return Err(wrong_parent("initiative"));
            }
            if let Some(id) = arg_str(&args, "id") {
                return if kind == "project" {
                    Ok(project_update_json(
                        &ws,
                        &app.get_project_update(ctx, id).await?,
                    ))
                } else {
                    Ok(initiative_update_json(
                        &ws,
                        &app.get_initiative_update(ctx, id).await?,
                    ))
                };
            }
            let now = app.clock.now();
            let time = |key: &str| {
                arg_str(&args, key)
                    .map(|s| parse_time_filter(key, s, now))
                    .transpose()
            };
            if kind == "project" {
                let (page, views) = app
                    .list_project_updates(
                        ctx,
                        ListProjectUpdatesInput {
                            project: arg_str(&args, "project").map(str::to_owned),
                            user: arg_str(&args, "user").map(str::to_owned),
                            include_archived: arg_bool(&args, "includeArchived"),
                            created_after: time("createdAt")?,
                            updated_after: time("updatedAt")?,
                            page: page_request(tool, &args)?,
                        },
                    )
                    .await?;
                Ok(envelope(
                    "statusUpdates",
                    tool,
                    &args,
                    &page,
                    views.iter().map(|v| project_update_json(&ws, v)).collect(),
                ))
            } else {
                let (page, views) = app
                    .list_initiative_updates(
                        ctx,
                        ListInitiativeUpdatesInput {
                            initiative: arg_str(&args, "initiative").map(str::to_owned),
                            user: arg_str(&args, "user").map(str::to_owned),
                            include_archived: arg_bool(&args, "includeArchived"),
                            created_after: time("createdAt")?,
                            updated_after: time("updatedAt")?,
                            page: page_request(tool, &args)?,
                        },
                    )
                    .await?;
                Ok(envelope(
                    "statusUpdates",
                    tool,
                    &args,
                    &page,
                    views
                        .iter()
                        .map(|v| initiative_update_json(&ws, v))
                        .collect(),
                ))
            }
        }
        "save_status_update" => {
            let h = health(&args)?;
            let body = arg_str(&args, "body");
            if kind == "project" && arg_str(&args, "initiative").is_some() {
                return Err(wrong_parent("project"));
            }
            if kind == "initiative" && arg_str(&args, "project").is_some() {
                return Err(wrong_parent("initiative"));
            }
            match arg_str(&args, "id") {
                Some(id) => {
                    if kind == "project" {
                        Ok(project_update_json(
                            &ws,
                            &app.edit_project_update(ctx, id, body, h).await?,
                        ))
                    } else {
                        Ok(initiative_update_json(
                            &ws,
                            &app.edit_initiative_update(ctx, id, body, h).await?,
                        ))
                    }
                }
                None => {
                    let body = body.filter(|b| !b.trim().is_empty()).ok_or_else(|| {
                        AppError::invalid_request("`body` is required")
                            .with_details(json!({ "path": "/body" }))
                    })?;
                    let hidden = arg_bool(&args, "isDiffHidden");
                    if kind == "project" {
                        let p = required(&args, "project")?;
                        Ok(project_update_json(
                            &ws,
                            &app.post_project_update(ctx, p, body, h, hidden).await?,
                        ))
                    } else {
                        let i = required(&args, "initiative")?;
                        Ok(initiative_update_json(
                            &ws,
                            &app.post_initiative_update(ctx, i, body, h, hidden).await?,
                        ))
                    }
                }
            }
        }
        "delete_status_update" => {
            let id = required(&args, "id")?;
            let (uuid, at) = if kind == "project" {
                app.delete_project_update(ctx, id)
                    .await
                    .map(|(u, t)| (u.to_string(), t))?
            } else {
                app.delete_initiative_update(ctx, id)
                    .await
                    .map(|(u, t)| (u.to_string(), t))?
            };
            Ok(json!({ "success": true, "id": uuid, "archivedAt": ts(at) }))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by status_updates"
        ))),
    }
}
