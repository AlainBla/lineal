//! Tool-Handler der Gruppe `attachments` (WP09): `prepare_attachment_upload`,
//! `create_attachment_from_upload`, `get_attachment`, `delete_attachment`, `extract_images`
//! (docs/07 §3.6). Der Upload selbst läuft außerhalb von MCP direkt gegen den Objektspeicher.

use super::comments::{arg_str, required_str};
use super::teams_users::ts;
use crate::envelope::{self, CONTENT_KEY};
use lineal_app::usecases::attachment::{AttachmentView, PrepareUploadInput};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Map, Value, json};

/// Größtes Objekt, dessen Inhalt `get_attachment` direkt ausliefert (5 MiB).
const MAX_CONTENT_BYTES: u64 = 5 * 1024 * 1024;

/// Maximale Zahl der Bilder, die `extract_images` bearbeitet.
const MAX_IMAGES: usize = 10;

fn attachment_json(v: &AttachmentView) -> Value {
    let a = &v.attachment;
    json!({
        "id": a.id.to_string(),
        "issue": v.issue_identifier,
        "title": a.title,
        "subtitle": envelope::opt_text(a.subtitle.as_deref()),
        "url": v.url,
        "contentType": a.content_type,
        "size": a.size,
        "createdAt": ts(a.created_at),
        "creator": v.creator.as_ref().map(|(id, name)| envelope::user_ref(id, name)),
    })
}

/// Bild-URLs aus Markdown (`![alt](url "title")`) und HTML (`<img src="url">`), in Reihenfolge.
pub(crate) fn image_urls(markdown: &str) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut from = 0;
    while let Some(p) = markdown[from..].find("![") {
        let start = from + p + 2;
        let Some(close) = markdown[start..].find("](") else {
            break;
        };
        let tail_start = start + close + 2;
        let Some(end) = markdown[tail_start..].find(')') else {
            break;
        };
        let raw = markdown[tail_start..tail_start + end].trim();
        let url = raw
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['<', '>']);
        if !url.is_empty() {
            found.push((start, url.to_owned()));
        }
        from = tail_start + end;
    }
    let lower = markdown.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find("<img") {
        let start = from + p;
        let Some(tag_end) = lower[start..].find('>') else {
            break;
        };
        let tag = &markdown[start..start + tag_end];
        let tag_lower = &lower[start..start + tag_end];
        if let Some(s) = tag_lower.find("src=") {
            let value = &tag[s + 4..];
            let url = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
                _ => value.split_whitespace().next().unwrap_or_default(),
            };
            if !url.is_empty() {
                found.push((start, url.to_owned()));
            }
        }
        from = start + tag_end;
    }
    found.sort_by_key(|(pos, _)| *pos);
    let mut urls: Vec<String> = Vec::new();
    for (_, u) in found {
        if !urls.contains(&u) {
            urls.push(u);
        }
    }
    urls
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "prepare_attachment_upload" => {
            let size = args.get("size").and_then(Value::as_i64).ok_or_else(|| {
                AppError::invalid_request("`size` must be an integer")
                    .with_details(json!({ "path": "/size" }))
            })?;
            let prepared = app
                .prepare_attachment_upload(
                    ctx,
                    PrepareUploadInput {
                        issue: Some(required_str(&args, "issue")?.to_owned()),
                        filename: required_str(&args, "filename")?.to_owned(),
                        content_type: required_str(&args, "contentType")?.to_owned(),
                        size,
                        title: arg_str(&args, "title")?.map(str::to_owned),
                        subtitle: arg_str(&args, "subtitle")?.map(str::to_owned),
                    },
                )
                .await?;
            let headers: Map<String, Value> = prepared
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            Ok(json!({
                "uploadRequest": { "url": prepared.url, "method": prepared.method, "headers": headers },
                "assetUrl": prepared.asset_url,
                "expiresAt": ts(prepared.expires_at),
                "suggested": {
                    "title": prepared.suggested_title,
                    "subtitle": envelope::opt_text(prepared.suggested_subtitle.as_deref()),
                },
            }))
        }
        "create_attachment_from_upload" => {
            let m = app
                .create_attachment_from_upload(
                    ctx,
                    required_str(&args, "issue")?,
                    required_str(&args, "assetUrl")?,
                    arg_str(&args, "title")?,
                    arg_str(&args, "subtitle")?,
                )
                .await?;
            Ok(attachment_json(&m.view))
        }
        "get_attachment" => {
            let id = required_str(&args, "id")?;
            let v = app.get_attachment(ctx, id).await?;
            // `format: "content"` (Default): Bilder als image-Element, Text als Text-Inhalt
            // (docs/07 §3.6); alles andere und Links liefern Metadaten mit Download-URL.
            if arg_str(&args, "format")? != Some("url")
                && v.expires_at.is_some()
                && let Some((bytes, content_type)) = app
                    .read_attachment_content(ctx, id, MAX_CONTENT_BYTES)
                    .await?
            {
                let mime = content_type
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                let is_text = mime.starts_with("text/")
                    || mime == "application/json"
                    || mime.ends_with("+json");
                let mut meta = attachment_json(&v);
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("expiresAt".into(), json!(v.expires_at.map(ts)));
                }
                if mime.starts_with("image/") {
                    use base64::Engine;
                    return Ok(json!({
                        CONTENT_KEY: [
                            { "type": "text", "text": meta.to_string() },
                            {
                                "type": "image",
                                "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                                "mimeType": mime,
                            },
                        ]
                    }));
                }
                if is_text && let Ok(text) = String::from_utf8(bytes) {
                    return Ok(json!({
                        CONTENT_KEY: [
                            { "type": "text", "text": meta.to_string() },
                            { "type": "text", "text": text },
                        ]
                    }));
                }
            }
            let mut out = attachment_json(&v);
            if let Some(obj) = out.as_object_mut() {
                obj.insert("expiresAt".into(), json!(v.expires_at.map(ts)));
                // Der Dateiinhalt wird nicht durch den MCP-Server geschleust: Download über `url`.
                if arg_str(&args, "format")? != Some("url") && v.expires_at.is_some() {
                    obj.insert("note".into(), json!("Binary content; download via url"));
                }
            }
            Ok(out)
        }
        "delete_attachment" => {
            let d = app
                .delete_attachment(ctx, required_str(&args, "id")?)
                .await?;
            Ok(envelope::success(d.id))
        }
        "extract_images" => {
            let markdown = required_str(&args, "markdown")?;
            let urls = image_urls(markdown);
            let prefix = app.asset_prefix();
            let own: Vec<String> = urls
                .iter()
                .take(MAX_IMAGES)
                .filter(|u| u.starts_with(&prefix))
                .cloned()
                .collect();
            let resolved = app.resolve_assets(ctx, &own).await?;
            let images: Vec<Value> = urls
                .iter()
                .enumerate()
                .map(|(i, url)| {
                    if i >= MAX_IMAGES {
                        json!({ "url": url, "status": "skipped_limit" })
                    } else if !url.starts_with(&prefix) {
                        json!({ "url": url, "status": "skipped_external" })
                    } else if let Some(a) = resolved.iter().find(|a| &a.asset_url == url) {
                        json!({
                            "url": url,
                            "status": "ok",
                            "contentType": a.content_type,
                            "downloadUrl": a.download_url,
                            "expiresAt": ts(a.expires_at),
                        })
                    } else {
                        json!({ "url": url, "status": "not_found" })
                    }
                })
                .collect();
            Ok(json!({
                CONTENT_KEY: [{ "type": "text", "text": json!({ "images": images }).to_string() }]
            }))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` does not belong to the attachments group"
        ))),
    }
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
