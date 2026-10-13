//! usecases – Anhänge (docs/03 §7): Link-Anhänge (Dedupe pro URL), Upload-Flow
//! prepare → PUT → finalize über `BlobStore`, signierte Downloads, Löschen.

use super::comment::{Deleted, actor_user};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::attachment::{Attachment, IssueRef, NewLink, NewUpload, Reservation};
use crate::resolve;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::*;
use serde_json::{Value, json};
use std::time::Duration;

/// Gültigkeit vorsignierter Upload-URLs (docs/03 §7.2).
pub const PUT_TTL: Duration = Duration::from_secs(60);
/// Gültigkeit signierter Download-URLs.
pub const GET_TTL: Duration = Duration::from_secs(5 * 60);
/// Gültigkeit einer Upload-Reservierung bis zum Finalisieren (docs/07 §3.6).
pub const RESERVATION_TTL: Duration = Duration::from_secs(60 * 60);
/// Obergrenze der Dateigröße (exklusiv): 2 GiB (docs/03 §7.2).
pub const MAX_UPLOAD_BYTES: i64 = 2 * 1024 * 1024 * 1024;
/// Pfadpräfix der dauerhaften (nicht öffentlichen) Asset-URLs.
pub const ASSET_PATH: &str = "assets";

/// Eingabe von `prepare_attachment_upload` / `fileUpload`.
#[derive(Debug, Clone)]
pub struct PrepareUploadInput {
    /// Issue (Referenz); `None` für Uploads ohne Ziel (z. B. Bilder in Markdown).
    pub issue: Option<String>,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
}

/// Antwort von `prepare`.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedUpload {
    pub url: String,
    pub method: &'static str,
    /// Beim PUT unverändert mitzusenden.
    pub headers: Vec<(String, String)>,
    pub asset_url: String,
    pub expires_at: DateTime<Utc>,
    pub suggested_title: String,
    pub suggested_subtitle: Option<String>,
}

/// Eine Datei des eigenen Workspaces, die per `assetUrl` referenziert wird (Kommentare, Markdown).
#[derive(Debug, Clone, PartialEq)]
pub struct AssetInfo {
    /// Die referenzierte (dauerhafte) URL.
    pub asset_url: String,
    pub title: String,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    /// Signierte Download-URL (`GET_TTL`).
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
}

/// Anhang mit Anzeigedaten.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentView {
    pub attachment: Attachment,
    pub issue_identifier: String,
    pub creator: Option<(UserId, String)>,
    /// Hochgeladene Dateien: signierte Download-URL; Links: die URL unverändert.
    pub url: String,
    /// Ablauf der signierten URL (nur bei Uploads).
    pub expires_at: Option<DateTime<Utc>>,
}

/// Ergebnis einer Schreiboperation.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentMutation {
    pub view: AttachmentView,
    pub last_sync_id: i64,
    /// `false`, wenn ein bestehender Link nur aktualisiert wurde.
    pub created: bool,
}

/// Eingabe für [`App::link_attachment`].
#[derive(Debug, Clone, Default)]
pub struct LinkAttachmentInput {
    pub issue: String,
    pub url: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub metadata: Option<Value>,
}

fn not_found_issue(reference: &str) -> AppError {
    AppError::not_found("Could not find referenced Issue.")
        .with_details(json!({ "reference": reference, "type": "Issue" }))
}

fn upload_error(reason: &str, message: &str) -> AppError {
    AppError::invalid_request(message).with_details(json!({ "reason": reason }))
}

fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// `type/subtype` nach RFC 6838 (ohne Parameter), klein geschrieben.
fn validate_content_type(raw: &str) -> AppResult<String> {
    let ct = essence(raw);
    let token = |s: &str| {
        !s.is_empty()
            && s.len() <= 127
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$&^_.+-".contains(c))
    };
    match ct.split_once('/') {
        Some((t, s)) if token(t) && token(s) => Ok(ct),
        _ => Err(AppError::invalid_request("Invalid content type")
            .with_details(json!({ "field": "contentType" }))),
    }
}

fn validate_filename(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    let bad = name.is_empty()
        || name.chars().count() > 255
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'));
    if bad {
        return Err(AppError::invalid_request("Invalid filename")
            .with_details(json!({ "field": "filename" })));
    }
    Ok(name.to_owned())
}

/// Normalisiert eine Link-URL (docs/03 §7.1): Schema und Host klein, Fragment entfernt,
/// abschließender `/` entfernt. Nur `http` und `https`.
pub fn normalize_url(raw: &str) -> AppResult<String> {
    let bad = || {
        AppError::invalid_request("url must be a valid http(s) URL")
            .with_details(json!({ "field": "url" }))
    };
    let raw = raw.trim();
    if raw.is_empty()
        || raw.chars().count() > 2048
        || raw.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad());
    }
    let without_fragment = raw.split('#').next().unwrap_or_default();
    let (scheme, rest) = without_fragment.split_once("://").ok_or_else(bad)?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(bad());
    }
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.starts_with('@') {
        return Err(bad());
    }
    let tail = tail.strip_suffix('/').unwrap_or(tail);
    Ok(format!(
        "{scheme}://{}{tail}",
        authority.to_ascii_lowercase()
    ))
}

fn disposition(filename: &str) -> String {
    let safe: String = filename
        .chars()
        .map(|c| if matches!(c, '"' | '\\') { '_' } else { c })
        .collect();
    format!("attachment; filename=\"{safe}\"")
}

impl App {
    /// Präfix der dauerhaften Asset-URLs dieses Servers (`{base_url}/assets/`).
    pub fn asset_prefix(&self) -> String {
        format!(
            "{}/{ASSET_PATH}/",
            self.config.base_url.trim_end_matches('/')
        )
    }

    async fn attachment_view(
        &self,
        tx: &mut dyn Tx,
        attachment: Attachment,
        issue_identifier: String,
    ) -> AppResult<AttachmentView> {
        let creator = match attachment.creator_id {
            Some(uid) => tx.users().get(uid).await?.map(|u| (u.id, u.name)),
            None => None,
        };
        let (url, expires_at) = match (&attachment.storage_key, attachment.source_type.as_deref()) {
            (Some(key), Some("upload")) => (
                self.blobs.signed_get_url(key, GET_TTL).await?,
                Some(self.clock.now() + ChronoDuration::seconds(GET_TTL.as_secs() as i64)),
            ),
            _ => (attachment.url.clone(), None),
        };
        Ok(AttachmentView {
            attachment,
            issue_identifier,
            creator,
            url,
            expires_at,
        })
    }

    async fn issue_for_attachments(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        reference: &str,
        action: Action,
    ) -> AppResult<IssueRef> {
        let issue = tx
            .attachments()
            .find_issue(reference)
            .await?
            .ok_or_else(|| not_found_issue(reference))?;
        resolve::issue::authorize_issue(tx, snap, ctx, issue.id, issue.team_id, action)
            .await
            .map_err(|e| {
                if e.kind == crate::error::ErrorKind::NotFound {
                    not_found_issue(reference)
                } else {
                    e
                }
            })?;
        Ok(issue)
    }

    /// Schritt 1 des Uploads: Zugriff, Größe und Typ prüfen, Schlüssel reservieren, PUT-URL signieren.
    pub async fn prepare_attachment_upload(
        &self,
        ctx: &RequestCtx,
        input: PrepareUploadInput,
    ) -> AppResult<PreparedUpload> {
        let filename = validate_filename(&input.filename)?;
        let content_type = validate_content_type(&input.content_type)?;
        if input.size <= 0 {
            return Err(AppError::invalid_request("size must be greater than 0")
                .with_details(json!({ "field": "size" })));
        }
        if input.size >= MAX_UPLOAD_BYTES {
            return Err(
                AppError::limit_exceeded("File size must be smaller than 2 GB").with_details(
                    json!({ "limit": "maxUploadBytes", "maxBytes": MAX_UPLOAD_BYTES }),
                ),
            );
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = match &input.issue {
            Some(r) => Some(
                self.issue_for_attachments(&mut *tx, &snap, ctx, r, Action::ManageAttachments)
                    .await?,
            ),
            None => {
                // Ohne Issue: mindestens Schreibzugriff in einem sichtbaren Team nötig.
                let visible = snap.visible_team_ids();
                let mut allowed = false;
                for t in visible {
                    if snap.require_team(ctx, Action::ManageAttachments, t).is_ok() {
                        allowed = true;
                        break;
                    }
                }
                if !allowed {
                    return Err(AppError::forbidden("Not allowed to upload files.")
                        .with_details(json!({ "reason": "role" })));
                }
                None
            }
        };
        let now = self.clock.now();
        let key = format!(
            "{}/{}/{}",
            ctx.org_id.0,
            self.ids.new_uuid(),
            self.ids.new_uuid()
        );
        let put_url = self
            .blobs
            .signed_put_url_sized(
                &key,
                &content_type,
                u64::try_from(input.size).unwrap_or(0),
                PUT_TTL,
            )
            .await?;
        let title = input
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| filename.clone());
        tx.attachments()
            .insert_reservation(&Reservation {
                id: self.ids.new_uuid(),
                issue_id: issue.as_ref().map(|i| i.id),
                creator_id: actor_user(ctx),
                storage_key: key.clone(),
                filename: filename.clone(),
                content_type: content_type.clone(),
                size: input.size,
                title: input.title.clone(),
                subtitle: input.subtitle.clone(),
                expires_at: now + ChronoDuration::seconds(RESERVATION_TTL.as_secs() as i64),
                finalized_at: None,
                created_at: now,
            })
            .await?;
        tx.commit().await?;
        Ok(PreparedUpload {
            url: put_url,
            method: "PUT",
            headers: vec![
                ("content-type".into(), content_type),
                (
                    "x-content-length-range".into(),
                    format!("{0},{0}", input.size),
                ),
                (
                    "cache-control".into(),
                    "public, max-age=31536000".to_owned(),
                ),
                ("content-disposition".into(), disposition(&filename)),
            ],
            asset_url: format!("{}{key}", self.asset_prefix()),
            expires_at: now + ChronoDuration::seconds(PUT_TTL.as_secs() as i64),
            suggested_title: title,
            suggested_subtitle: input.subtitle,
        })
    }

    /// Schritt 3 des Uploads: prüft Reservierung (Workspace, Issue, Ersteller, Ablauf, einmalig) und
    /// Objekt (Existenz, Größe, Typ) und legt den Anhang an.
    pub async fn create_attachment_from_upload(
        &self,
        ctx: &RequestCtx,
        issue: &str,
        asset_url: &str,
        title: Option<&str>,
        subtitle: Option<&str>,
    ) -> AppResult<AttachmentMutation> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = self
            .issue_for_attachments(&mut *tx, &snap, ctx, issue, Action::ManageAttachments)
            .await?;

        let key = asset_url
            .trim()
            .strip_prefix(&self.asset_prefix())
            .ok_or_else(|| {
                upload_error(
                    "invalid_asset_url",
                    "assetUrl does not belong to this workspace",
                )
            })?;
        let org_segment = ctx.org_id.0.to_string();
        let parts: Vec<&str> = key.split('/').collect();
        if parts.len() != 3 || parts[0] != org_segment || parts.iter().any(|p| p.is_empty()) {
            return Err(upload_error(
                "invalid_asset_url",
                "assetUrl does not belong to this workspace",
            ));
        }
        let reservation = tx
            .attachments()
            .reservation_by_key(key)
            .await?
            .ok_or_else(|| {
                upload_error(
                    "upload_unknown",
                    "Upload not found. Rerun prepare_attachment_upload and upload again",
                )
            })?;
        if reservation.issue_id.is_some_and(|i| i != issue.id) {
            return Err(upload_error(
                "upload_issue_mismatch",
                "assetUrl was prepared for a different issue",
            ));
        }
        if reservation.creator_id != actor_user(ctx) {
            return Err(upload_error(
                "upload_not_owned",
                "assetUrl was prepared by a different user",
            ));
        }
        if reservation.finalized_at.is_some() {
            return Err(AppError::conflict("The upload was already finalized")
                .with_details(json!({ "reason": "upload_finalized" })));
        }
        let now = self.clock.now();
        if reservation.expires_at < now {
            return Err(upload_error(
                "upload_expired",
                "The upload reservation expired. Rerun prepare_attachment_upload and upload again",
            ));
        }
        let stored = self.blobs.head(key).await?.ok_or_else(|| {
            upload_error(
                "upload_missing",
                "Upload not found. Rerun prepare_attachment_upload and upload again",
            )
        })?;
        if i64::try_from(stored.size).ok() != Some(reservation.size) {
            return Err(upload_error(
                "upload_size_mismatch",
                "The uploaded file size differs from the announced size",
            )
            .with_details(json!({
                "reason": "upload_size_mismatch",
                "expected": reservation.size,
                "actual": stored.size,
            })));
        }
        if essence(&stored.content_type) != essence(&reservation.content_type) {
            return Err(upload_error(
                "upload_type_mismatch",
                "The uploaded content type differs from the announced type",
            ));
        }
        if !tx
            .attachments()
            .finalize_reservation(reservation.id, now)
            .await?
        {
            return Err(AppError::conflict("The upload was already finalized")
                .with_details(json!({ "reason": "upload_finalized" })));
        }
        let title = title
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
            .or(reservation.title.clone())
            .unwrap_or_else(|| reservation.filename.clone());
        let attachment = tx
            .attachments()
            .insert_upload(&NewUpload {
                id: AttachmentId(self.ids.new_uuid()),
                issue_id: issue.id,
                url: asset_url.trim().to_owned(),
                title,
                subtitle: subtitle.map(str::to_owned).or(reservation.subtitle.clone()),
                content_type: reservation.content_type.clone(),
                size: reservation.size,
                storage_key: key.to_owned(),
                creator_id: actor_user(ctx),
                now,
            })
            .await?;
        tx.attachments()
            .add_history(
                issue.id,
                actor_user(ctx),
                attachment.id,
                "attachmentAdded",
                now,
            )
            .await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::AttachmentCreated {
                    attachment_id: attachment.id,
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        let view = self
            .attachment_view(&mut *tx, attachment, issue.identifier)
            .await?;
        tx.commit().await?;
        Ok(AttachmentMutation {
            view,
            last_sync_id,
            created: true,
        })
    }

    /// Hängt einen Link an (Upsert pro Issue und normalisierter URL).
    pub async fn link_attachment(
        &self,
        ctx: &RequestCtx,
        input: LinkAttachmentInput,
    ) -> AppResult<AttachmentMutation> {
        let url = normalize_url(&input.url)?;
        let title = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map_or_else(|| url.clone(), str::to_owned);
        if title.chars().count() > 1000 {
            return Err(AppError::invalid_request("title is too long")
                .with_details(json!({ "field": "title" })));
        }
        if let Some(m) = &input.metadata
            && !m.is_object()
        {
            return Err(AppError::invalid_request("metadata must be an object")
                .with_details(json!({ "field": "metadata" })));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = self
            .issue_for_attachments(
                &mut *tx,
                &snap,
                ctx,
                &input.issue,
                Action::ManageAttachments,
            )
            .await?;
        let now = self.clock.now();
        let (attachment, created) = tx
            .attachments()
            .upsert_link(&NewLink {
                id: AttachmentId(self.ids.new_uuid()),
                issue_id: issue.id,
                url,
                title,
                subtitle: input.subtitle.clone().filter(|s| !s.trim().is_empty()),
                metadata: input.metadata.clone(),
                creator_id: actor_user(ctx),
                now,
            })
            .await?;
        if created {
            tx.attachments()
                .add_history(
                    issue.id,
                    actor_user(ctx),
                    attachment.id,
                    "attachmentAdded",
                    now,
                )
                .await?;
        }
        let event = if created {
            DomainEvent::AttachmentCreated {
                attachment_id: attachment.id,
            }
        } else {
            DomainEvent::AttachmentUpdated {
                attachment_id: attachment.id,
            }
        };
        tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
        let last_sync_id = tx.last_sync_id().await?;
        let view = self
            .attachment_view(&mut *tx, attachment, issue.identifier)
            .await?;
        tx.commit().await?;
        Ok(AttachmentMutation {
            view,
            last_sync_id,
            created,
        })
    }

    /// Anhang lesen; Uploads mit signierter Download-URL (Signatur erst nach der Zugriffsprüfung).
    pub async fn get_attachment(&self, ctx: &RequestCtx, id: &str) -> AppResult<AttachmentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (attachment, issue) = self
            .load_attachment(&mut *tx, &snap, ctx, id, Action::ViewTeam)
            .await?;
        let view = self
            .attachment_view(&mut *tx, attachment, issue.identifier)
            .await?;
        Ok(view)
    }

    /// Inhalt eines hochgeladenen Anhangs (nach Zugriffsprüfung wie `get_attachment`) samt
    /// Content-Type; `None` bei Links, zu großen oder nicht lesbaren Dateien (B8.12).
    pub async fn read_attachment_content(
        &self,
        ctx: &RequestCtx,
        id: &str,
        max_bytes: u64,
    ) -> AppResult<Option<(Vec<u8>, String)>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (attachment, _issue) = self
            .load_attachment(&mut *tx, &snap, ctx, id, Action::ViewTeam)
            .await?;
        drop(tx);
        let (Some(key), Some("upload")) =
            (&attachment.storage_key, attachment.source_type.as_deref())
        else {
            return Ok(None);
        };
        let content_type = attachment.content_type.clone().unwrap_or_default();
        Ok(self
            .blobs
            .read(key, max_bytes)
            .await?
            .map(|bytes| (bytes, content_type)))
    }

    /// Bildet `assetUrl`s des eigenen Workspaces auf signierte Download-URLs ab. Nur lesbare Dateien:
    /// Issue-Anhänge nach Zugriffsprüfung auf das Issue sowie Uploads ohne Ziel-Issue (z. B. Bilder
    /// in Markdown). Fremde, unbekannte oder nicht lesbare URLs fehlen im Ergebnis.
    pub async fn resolve_assets(
        &self,
        ctx: &RequestCtx,
        urls: &[String],
    ) -> AppResult<Vec<AssetInfo>> {
        if urls.is_empty() {
            return Ok(vec![]);
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let prefix = self.asset_prefix();
        let org_segment = ctx.org_id.0.to_string();
        let now = self.clock.now();
        let mut out = Vec::new();
        for url in urls {
            let url = url.trim();
            let Some(key) = url.strip_prefix(&prefix) else {
                continue;
            };
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() != 3 || parts[0] != org_segment {
                continue;
            }
            let (title, content_type, size) =
                if let Some(a) = tx.attachments().find_by_storage_key(key).await? {
                    let Some(issue) = tx.attachments().issue_ref(a.issue_id).await? else {
                        continue;
                    };
                    if resolve::issue::authorize_issue(
                        &mut *tx,
                        &snap,
                        ctx,
                        issue.id,
                        issue.team_id,
                        Action::ViewTeam,
                    )
                    .await
                    .is_err()
                    {
                        continue;
                    }
                    (a.title, a.content_type, a.size)
                } else if let Some(r) = tx.attachments().reservation_by_key(key).await? {
                    if r.issue_id.is_some() {
                        continue;
                    }
                    (r.filename, Some(r.content_type), Some(r.size))
                } else {
                    continue;
                };
            out.push(AssetInfo {
                asset_url: url.to_owned(),
                title,
                content_type,
                size,
                download_url: self.blobs.signed_get_url(key, GET_TTL).await?,
                expires_at: now + ChronoDuration::seconds(GET_TTL.as_secs() as i64),
            });
        }
        Ok(out)
    }

    /// Alle Anhänge eines Issues (für `get_issue`).
    pub async fn list_issue_attachments(
        &self,
        ctx: &RequestCtx,
        issue: &str,
    ) -> AppResult<Vec<AttachmentView>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = self
            .issue_for_attachments(&mut *tx, &snap, ctx, issue, Action::ViewTeam)
            .await?;
        let rows = tx.attachments().list_for_issue(issue.id).await?;
        let mut views = Vec::with_capacity(rows.len());
        for a in rows {
            views.push(
                self.attachment_view(&mut *tx, a, issue.identifier.clone())
                    .await?,
            );
        }
        Ok(views)
    }

    async fn load_attachment(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        id: &str,
        action: Action,
    ) -> AppResult<(Attachment, IssueRef)> {
        let not_found = || {
            AppError::not_found("Could not find referenced Attachment.")
                .with_details(json!({ "reference": id, "type": "Attachment" }))
        };
        let aid: AttachmentId = id.trim().parse().map_err(|_| not_found())?;
        let attachment = tx.attachments().get(aid).await?.ok_or_else(not_found)?;
        let issue = tx
            .attachments()
            .issue_ref(attachment.issue_id)
            .await?
            .ok_or_else(not_found)?;
        resolve::issue::authorize_issue(tx, snap, ctx, issue.id, issue.team_id, action)
            .await
            .map_err(|e| {
                if e.kind == crate::error::ErrorKind::NotFound {
                    not_found()
                } else {
                    e
                }
            })?;
        Ok((attachment, issue))
    }

    /// Entfernt einen Anhang (weich; das Objekt räumt der Aufräumjob).
    pub async fn delete_attachment(&self, ctx: &RequestCtx, id: &str) -> AppResult<Deleted> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (attachment, issue) = self
            .load_attachment(&mut *tx, &snap, ctx, id, Action::ManageAttachments)
            .await?;
        // Löschen: Ersteller, Team-Owner oder Workspace-Admin/Owner (docs/03 §7).
        let is_creator =
            attachment.creator_id.is_some() && attachment.creator_id == actor_user(ctx);
        let is_admin = matches!(
            ctx.actor,
            crate::context::Actor::System
                | crate::context::Actor::User {
                    role: lineal_domain::enums::UserRole::Owner
                        | lineal_domain::enums::UserRole::Admin,
                    ..
                }
        );
        if !is_creator && !is_admin && !snap.view(issue.team_id)?.is_team_owner {
            return Err(AppError::forbidden(
                "Only the creator, a team owner or an admin can delete an attachment.",
            )
            .with_details(json!({ "reason": "not_creator" })));
        }
        let now = self.clock.now();
        tx.attachments().archive(attachment.id, now).await?;
        tx.attachments()
            .add_history(
                issue.id,
                actor_user(ctx),
                attachment.id,
                "attachmentRemoved",
                now,
            )
            .await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::AttachmentRemoved {
                    attachment_id: attachment.id,
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        tx.commit().await?;
        Ok(Deleted {
            id: attachment.id.0,
            last_sync_id,
        })
    }
}

/// Frist nach Ablauf einer Reservierung, bis ihr ungenutztes Objekt gelöscht wird.
pub const RESERVATION_GRACE: ChronoDuration = ChronoDuration::days(1);

impl App {
    /// Löscht Blobs best effort (Fehler werden geloggt, nicht weitergereicht: die Datenbankzeilen
    /// sind dann schon weg und ein Rest-Blob ist nur Speicherplatz). Liefert die Anzahl gelöschter.
    pub async fn delete_blobs(&self, keys: &[String]) -> u64 {
        let mut deleted = 0;
        for key in keys {
            match self.blobs.delete(key).await {
                Ok(()) => deleted += 1,
                Err(e) => tracing::warn!(error = %e, "blob delete failed"),
            }
        }
        deleted
    }

    /// Periodisch: räumt abgelaufene, nie finalisierte Upload-Reservierungen samt Blob ab.
    pub async fn run_upload_purge(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let cutoff = now - RESERVATION_GRACE;
        let mut sys = self.store.begin_system().await?;
        let expired = sys
            .attachments()
            .list_expired_reservations(cutoff, 500)
            .await?;
        drop(sys);
        let mut by_org: std::collections::BTreeMap<uuid::Uuid, Vec<Reservation>> =
            Default::default();
        for (org, r) in expired {
            by_org.entry(org.0).or_default().push(r);
        }
        let mut total = 0;
        for (org, list) in by_org {
            let ids: Vec<_> = list.iter().map(|r| r.id).collect();
            let keys: Vec<String> = list.into_iter().map(|r| r.storage_key).collect();
            let mut tx = self.store.begin(OrgId(org)).await?;
            tx.attachments().delete_reservations(&ids).await?;
            let orphans = tx.attachments().unreferenced_storage_keys(&keys).await?;
            tx.commit().await?;
            self.delete_blobs(&orphans).await;
            total += ids.len() as u64;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized() {
        assert_eq!(
            normalize_url("HTTPS://Example.COM/Path/?q=1#frag").unwrap(),
            "https://example.com/Path/?q=1"
        );
        assert_eq!(
            normalize_url("https://example.com/").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            normalize_url("https://example.com/a/").unwrap(),
            "https://example.com/a"
        );
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("ftp://x.org/a").is_err());
        assert!(normalize_url("https:///a").is_err());
        assert!(normalize_url("https://a b.com").is_err());
    }

    #[test]
    fn content_type_and_filename_are_validated() {
        assert_eq!(
            validate_content_type("Image/PNG; x=1").unwrap(),
            "image/png"
        );
        assert!(validate_content_type("nonsense").is_err());
        assert!(validate_content_type("a/b c").is_err());
        assert!(validate_filename("").is_err());
        assert!(validate_filename("../x").is_err());
        assert!(validate_filename("shot.png").is_ok());
    }
}
