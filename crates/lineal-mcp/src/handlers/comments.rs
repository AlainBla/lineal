//! Tool-Handler der Gruppe `comments` (WP09): `list_comments`, `save_comment`, `delete_comment`
//! (docs/07 §3.5). Die Eingabe wurde bereits gegen das Snapshot-Schema validiert; die Handler
//! prüfen die Regeln, die ein Schema nicht ausdrücken kann (genau ein Elternobjekt …).

use super::teams_users::ts;
use crate::envelope::{self, decode_cursor, encode_cursor, parse_limit, query_fingerprint};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::comment::ParentKind;
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::attachment::AssetInfo;
use lineal_app::usecases::comment::{
    CommentView, CreateCommentInput, ListCommentsInput, ParentSpec,
};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(
            AppError::invalid_request(format!("`{key}` must be a string"))
                .with_details(json!({ "path": format!("/{key}") })),
        ),
    }
}

pub(crate) fn required_str<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    arg_str(args, key)?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            AppError::invalid_request(format!("`{key}` is required"))
                .with_details(json!({ "path": format!("/{key}") }))
        })
}

/// Die Parent-Felder aus den Argumenten; höchstens eines darf gesetzt sein.
fn parent_from_args(args: &Value) -> AppResult<Option<ParentSpec>> {
    let mut found: Vec<ParentSpec> = Vec::new();
    if let Some(v) = arg_str(args, "issueId")? {
        found.push(ParentSpec::Issue(v.to_owned()));
    }
    if let Some(v) = arg_str(args, "projectId")? {
        found.push(ParentSpec::Project(v.to_owned()));
    }
    if let Some(v) = arg_str(args, "initiativeId")? {
        found.push(ParentSpec::Initiative(v.to_owned()));
    }
    if let Some(v) = arg_str(args, "documentId")? {
        found.push(ParentSpec::Document(v.to_owned()));
    }
    if let Some(v) = arg_str(args, "milestoneId")? {
        found.push(ParentSpec::Milestone(v.to_owned()));
    }
    if let Some(v) = arg_str(args, "statusUpdateId")? {
        let kind = match arg_str(args, "statusUpdateType")? {
            Some("project") => Some(ParentKind::ProjectUpdate),
            Some("initiative") => Some(ParentKind::InitiativeUpdate),
            Some(_) => {
                return Err(AppError::invalid_request(
                    "statusUpdateType must be `project` or `initiative`",
                ));
            }
            None => None,
        };
        found.push(ParentSpec::StatusUpdate {
            id: v.to_owned(),
            kind,
        });
    } else if arg_str(args, "statusUpdateType")?.is_some() {
        return Err(AppError::invalid_request(
            "statusUpdateType is only valid together with statusUpdateId",
        ));
    }
    if found.len() > 1 {
        return Err(AppError::invalid_request(
            "Provide exactly one of issueId, projectId, initiativeId, documentId, milestoneId, or statusUpdateId",
        ));
    }
    Ok(found.pop())
}

/// Alle Asset-URLs dieses Servers in einem Text (Markdown).
pub(crate) fn asset_urls_in(text: &str, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(prefix) {
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | ']' | '<'))
            .unwrap_or(tail.len());
        let url = tail[..end].to_owned();
        if !out.contains(&url) {
            out.push(url);
        }
        rest = &tail[end.max(prefix.len())..];
    }
    out
}

/// Listeneintrag / gespeicherter Kommentar (docs/07 §3.5).
fn comment_json(
    v: &CommentView,
    ws_url: &str,
    files: &HashMap<String, AssetInfo>,
    prefix: &str,
) -> Value {
    let c = &v.comment;
    let attachments: Vec<Value> = asset_urls_in(&c.body, prefix)
        .iter()
        .filter_map(|u| files.get(u))
        .map(|a| json!({ "title": a.title, "url": a.download_url, "contentType": a.content_type }))
        .collect();
    let reactions: Vec<Value> = v
        .reactions
        .iter()
        .map(|g| json!({ "emoji": g.emoji, "count": g.count }))
        .collect();
    let url = v
        .issue_identifier
        .as_ref()
        .map(|ident| format!("{ws_url}/issue/{ident}#comment-{}", c.id));
    json!({
        "id": c.id.to_string(),
        "body": c.body,
        "parentId": c.parent_id.map(|p| p.to_string()),
        "quotedText": c.quoted_text,
        "resolvedAt": c.resolved_at.map(ts),
        "author": v.author.as_ref().map(|a| envelope::user_ref(a.id, &a.name)),
        "onBehalfOf": null,
        "attachments": attachments,
        "reactions": reactions,
        "url": url,
        "createdAt": ts(c.created_at),
        "updatedAt": ts(c.updated_at),
        "editedAt": c.edited_at.map(ts),
    })
}

async fn render(app: &App, ctx: &RequestCtx, views: &[CommentView]) -> AppResult<Vec<Value>> {
    let org = app.get_workspace(ctx).await?;
    let ws_url = app.workspace_url(&org);
    let prefix = app.asset_prefix();
    let urls: Vec<String> = views
        .iter()
        .flat_map(|v| asset_urls_in(&v.comment.body, &prefix))
        .collect();
    let files: HashMap<String, AssetInfo> = app
        .resolve_assets(ctx, &urls)
        .await?
        .into_iter()
        .map(|a| (a.asset_url.clone(), a))
        .collect();
    Ok(views
        .iter()
        .map(|v| comment_json(v, &ws_url, &files, &prefix))
        .collect())
}

fn cursor_key(c: &PageCursor) -> Value {
    Value::String(c.key.to_rfc3339_opts(SecondsFormat::Micros, true))
}

fn page_cursor(raw: &str, fingerprint: &str) -> AppResult<PageCursor> {
    let bad = || AppError::invalid_request("Invalid cursor");
    let pos = decode_cursor(raw, fingerprint)?;
    let key = pos
        .key
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .ok_or_else(bad)?;
    let id = Uuid::parse_str(&pos.id).map_err(|_| bad())?;
    Ok(PageCursor { key, id })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_comments" => {
            let parent = parent_from_args(&args)?.ok_or_else(|| {
                AppError::invalid_request(
                    "Provide exactly one of issueId, projectId, initiativeId, documentId, milestoneId, or statusUpdateId",
                )
            })?;
            let limit = parse_limit(&args, envelope::DEFAULT_LIMIT, envelope::MAX_LIMIT)?;
            let order_by = match arg_str(&args, "orderBy")? {
                Some("createdAt") => OrderBy::CreatedAt,
                _ => OrderBy::UpdatedAt,
            };
            let fingerprint = query_fingerprint(&args);
            let cursor = arg_str(&args, "cursor")?
                .map(|c| page_cursor(c, &fingerprint))
                .transpose()?;
            let page = app
                .list_comments(
                    ctx,
                    ListCommentsInput {
                        parent,
                        order_by,
                        cursor,
                        limit: limit as i64,
                    },
                )
                .await?;
            let next = page
                .next
                .as_ref()
                .map(|c| encode_cursor(&cursor_key(c), c.id, &fingerprint));
            let items = render(app, ctx, &page.items).await?;
            Ok(envelope::page("comments", items, next))
        }
        "save_comment" => {
            let body = required_str(&args, "body")?;
            let id = arg_str(&args, "id")?;
            let parent_id = arg_str(&args, "parentId")?;
            if id.is_some() && parent_id.is_some() {
                return Err(AppError::invalid_request(
                    "`parentId` can only be used when creating a comment, not together with `id`",
                ));
            }
            let mutation = if let Some(id) = id {
                if arg_str(&args, "statusUpdateType")?.is_some() {
                    return Err(AppError::invalid_request(
                        "statusUpdateType is rejected when `id` is provided",
                    ));
                }
                app.update_comment(ctx, id, body).await?
            } else if let Some(parent_id) = parent_id {
                if arg_str(&args, "statusUpdateType")?.is_some() {
                    return Err(AppError::invalid_request(
                        "statusUpdateType is rejected when `parentId` is provided",
                    ));
                }
                app.create_comment(
                    ctx,
                    CreateCommentInput {
                        reply_to: Some(parent_id.to_owned()),
                        body: body.to_owned(),
                        ..Default::default()
                    },
                )
                .await?
            } else {
                let parent = parent_from_args(&args)?.ok_or_else(|| {
                    AppError::invalid_request(
                        "Provide exactly one of issueId, projectId, initiativeId, documentId, milestoneId, or statusUpdateId (or parentId to reply)",
                    )
                })?;
                app.create_comment(
                    ctx,
                    CreateCommentInput {
                        parent: Some(parent),
                        body: body.to_owned(),
                        ..Default::default()
                    },
                )
                .await?
            };
            let mut rendered = render(app, ctx, std::slice::from_ref(&mutation.view)).await?;
            rendered
                .pop()
                .ok_or_else(|| AppError::internal("comment rendering failed"))
        }
        "delete_comment" => {
            let deleted = app.delete_comment(ctx, required_str(&args, "id")?).await?;
            Ok(envelope::success(deleted.id))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` does not belong to the comments group"
        ))),
    }
}

#[cfg(test)]
#[path = "comments_tests.rs"]
pub(crate) mod tests;
