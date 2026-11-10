//! Tool-Handler der Gruppe `milestones` (WP07): `list_milestones`, `get_milestone`,
//! `save_milestone` (docs/07 §3.8). `progress` ist 0..100.

use super::projects::{arg_str, milestone_json, required};
use crate::envelope::page;
use lineal_app::usecases::attachment::PrepareUploadInput;
use lineal_app::usecases::project::milestone::{OpenIssues, SaveMilestoneInput};
use lineal_app::usecases::project::pure::parse_date;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn ts(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_milestones" => {
            let (p, list) = app
                .list_milestones(ctx, required(&args, "project")?)
                .await?;
            Ok(page(
                "milestones",
                list.iter().map(|m| milestone_json(&p, m)).collect(),
                None,
            ))
        }
        "get_milestone" => {
            let (p, m) = app
                .get_milestone(ctx, required(&args, "project")?, required(&args, "query")?)
                .await?;
            let mut j = milestone_json(&p, &m);
            j["issueCount"] = json!(m.issue_count);
            j["completedIssueCount"] = json!(m.completed_issue_count);
            j["startedIssueCount"] = json!(m.started_issue_count);
            j["completedAt"] = json!(m.milestone.completed_at.map(ts));
            j["resultNote"] = json!(m.milestone.result_note);
            j["targetDateHistory"] = json!(
                m.date_changes
                    .iter()
                    .map(|c| json!({
                        "from": c.from_date.map(|d| d.to_string()),
                        "to": c.to_date.map(|d| d.to_string()),
                        "changedAt": ts(c.created_at),
                    }))
                    .collect::<Vec<_>>()
            );
            Ok(j)
        }
        "save_milestone" => {
            let target_date = match args.get("targetDate") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => {
                    let s = v.as_str().unwrap_or_default();
                    Some(Some(parse_date("targetDate", s)?))
                }
            };
            let (p, m) = app
                .save_milestone(
                    ctx,
                    SaveMilestoneInput {
                        project: required(&args, "project")?.to_owned(),
                        id: arg_str(&args, "id").map(str::to_owned),
                        name: arg_str(&args, "name").map(str::to_owned),
                        description: arg_str(&args, "description").map(str::to_owned),
                        target_date,
                        sort_order: args.get("sortOrder").and_then(Value::as_f64),
                    },
                )
                .await?;
            Ok(milestone_json(&p, &m))
        }
        "complete_milestone" => {
            let open = match (required(&args, "openIssues")?, arg_str(&args, "moveTo")) {
                ("keep", _) => OpenIssues::Keep,
                ("remove", _) => OpenIssues::Remove,
                ("move", Some(to)) => OpenIssues::Move(to.to_owned()),
                _ => {
                    return Err(AppError::invalid_request(
                        "openIssues must be keep, remove, or move with moveTo.",
                    ));
                }
            };
            let (p, m) = app
                .complete_milestone(
                    ctx,
                    required(&args, "project")?,
                    required(&args, "milestone")?,
                    open,
                    arg_str(&args, "note"),
                )
                .await?;
            Ok(milestone_json(&p, &m))
        }
        "reopen_milestone" => {
            let (p, m) = app
                .reopen_milestone(
                    ctx,
                    required(&args, "project")?,
                    required(&args, "milestone")?,
                )
                .await?;
            Ok(milestone_json(&p, &m))
        }
        "convert_milestone_to_project" => {
            let v = app
                .convert_milestone_to_project(
                    ctx,
                    required(&args, "project")?,
                    required(&args, "milestone")?,
                )
                .await?;
            Ok(json!({ "id": v.project.id.to_string(), "name": v.project.name }))
        }
        "get_milestone_resources" => {
            // Ohne `milestone`: Ressourcen/Anhänge des Projekts selbst (LIN-45).
            let r = app
                .list_resources(
                    ctx,
                    required(&args, "project")?,
                    arg_str(&args, "milestone"),
                )
                .await?;
            Ok(json!({
                "documents": r.documents.iter().map(|d| json!({
                    "id": d.id.to_string(), "title": d.title, "slugId": d.slug_id,
                })).collect::<Vec<_>>(),
                "links": r.links.iter().map(|l| json!({
                    "id": l.id.to_string(), "url": l.url, "title": l.label,
                })).collect::<Vec<_>>(),
                "files": r.files.iter().map(|f| json!({
                    "id": f.file.id.to_string(), "title": f.file.title,
                    "contentType": f.file.content_type, "size": f.file.size,
                    "url": f.download_url,
                })).collect::<Vec<_>>(),
            }))
        }
        "save_milestone_resource" => {
            let project = required(&args, "project")?;
            // Ohne `milestone`: Ressource/Anhang am Projekt selbst (LIN-45).
            let milestone = arg_str(&args, "milestone");
            let title = arg_str(&args, "title");
            let given = [
                arg_str(&args, "url"),
                arg_str(&args, "document"),
                arg_str(&args, "assetUrl"),
            ];
            match given {
                [Some(url), None, None] => {
                    app.add_resource_link(ctx, project, milestone, url, title.unwrap_or(url))
                        .await?;
                }
                [None, Some(document), None] => {
                    app.set_document_milestone(ctx, project, document, milestone)
                        .await?;
                }
                [None, None, Some(asset)] => {
                    app.add_resource_file(ctx, project, milestone, asset, title)
                        .await?;
                }
                _ => {
                    return Err(AppError::invalid_request(
                        "Provide exactly one of url, document or assetUrl.",
                    ));
                }
            }
            Ok(json!({ "success": true }))
        }
        "delete_milestone_resource" => {
            let project = required(&args, "project")?;
            let id = required(&args, "id")?;
            match required(&args, "type")? {
                "link" => app.remove_resource_link(ctx, project, id).await?,
                "file" => app.remove_resource_file(ctx, project, id).await?,
                _ => app.set_document_milestone(ctx, project, id, None).await?,
            }
            Ok(json!({ "success": true }))
        }
        "prepare_milestone_upload" => {
            let size = args.get("size").and_then(Value::as_i64).ok_or_else(|| {
                AppError::invalid_request("`size` must be an integer")
                    .with_details(json!({ "path": "/size" }))
            })?;
            let p = app
                .prepare_attachment_upload(
                    ctx,
                    PrepareUploadInput {
                        issue: None,
                        filename: required(&args, "filename")?.to_owned(),
                        content_type: required(&args, "contentType")?.to_owned(),
                        size,
                        title: None,
                        subtitle: None,
                    },
                )
                .await?;
            let headers: serde_json::Map<String, Value> = p
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            Ok(json!({
                "uploadRequest": { "url": p.url, "method": p.method, "headers": headers },
                "assetUrl": p.asset_url,
            }))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by milestones"
        ))),
    }
}
