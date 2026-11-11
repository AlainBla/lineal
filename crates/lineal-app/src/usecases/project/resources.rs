//! usecases – Ressourcen an Meilenstein und Projekt (docs/04 §3.5): Dokumente, Links, Dateien.
//! Die Rechte folgen dem Projekt; `milestone = None` meint das Projekt selbst.

use super::{emit, require_manage, writable};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::project::{ExternalLink, ProjectFile};
use crate::resolve;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::MilestoneId;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

const GET_TTL: Duration = Duration::from_secs(10 * 60);

/// Verweis auf ein Dokument.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRef {
    pub id: Uuid,
    pub title: String,
    pub slug_id: String,
}

/// Datei mit signierter Download-URL.
#[derive(Debug, Clone, PartialEq)]
pub struct FileView {
    pub file: ProjectFile,
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
}

/// Alle Ressourcen eines Bereichs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourcesView {
    pub documents: Vec<DocumentRef>,
    pub links: Vec<ExternalLink>,
    pub files: Vec<FileView>,
}

fn upload_error(reason: &str, message: &str) -> AppError {
    AppError::invalid_request(message).with_details(json!({ "reason": reason }))
}

impl App {
    async fn scope_milestone(
        &self,
        tx: &mut dyn Tx,
        project: crate::ports::repos::project::Project,
        milestone: Option<&str>,
    ) -> AppResult<Option<MilestoneId>> {
        match milestone {
            None => Ok(None),
            Some(m) => Ok(Some(
                resolve::project::resolve_milestone(tx, project.id, m)
                    .await?
                    .id,
            )),
        }
    }

    /// Ressourcen eines Meilensteins oder (`None`) die des Projekts selbst.
    pub async fn list_resources(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: Option<&str>,
    ) -> AppResult<ResourcesView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        let scope = self
            .scope_milestone(&mut *tx, r.project.clone(), milestone)
            .await?;
        let documents = tx
            .projects()
            .list_scoped_documents(r.project.id, scope)
            .await?
            .into_iter()
            .map(|(id, title, slug_id)| DocumentRef { id, title, slug_id })
            .collect();
        let links = tx.projects().list_scoped_links(r.project.id, scope).await?;
        let rows = tx.projects().list_scoped_files(r.project.id, scope).await?;
        let expires_at = self.clock.now() + ChronoDuration::seconds(GET_TTL.as_secs() as i64);
        let mut files = Vec::with_capacity(rows.len());
        for file in rows {
            let download_url = self
                .blobs
                .signed_get_url(&file.storage_key, GET_TTL)
                .await?;
            files.push(FileView {
                file,
                download_url,
                expires_at,
            });
        }
        Ok(ResourcesView {
            documents,
            links,
            files,
        })
    }

    /// Link hinzufügen; dieselbe URL im selben Bereich bleibt unverändert.
    pub async fn add_resource_link(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: Option<&str>,
        url: &str,
        title: &str,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let scope = self
            .scope_milestone(&mut *tx, r.project.clone(), milestone)
            .await?;
        let (url, title) = super::validate_link(url, title)?;
        tx.projects()
            .add_scoped_link(
                r.project.id,
                scope,
                &url,
                &title,
                resolve::user::actor_user_id(ctx),
                self.clock.now(),
            )
            .await?;
        self.emit_resources_changed(&mut *tx, ctx, r.project.id, scope)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Link entfernen.
    pub async fn remove_resource_link(
        &self,
        ctx: &RequestCtx,
        project: &str,
        link: &str,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let id = parse_id("link", link)?;
        if !tx.projects().remove_link(r.project.id, id).await? {
            return Err(AppError::not_found("Could not find the link."));
        }
        self.emit_resources_changed(&mut *tx, ctx, r.project.id, None)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ordnet ein Dokument des Projekts einem Meilenstein zu oder (`None`) löst es.
    pub async fn set_document_milestone(
        &self,
        ctx: &RequestCtx,
        project: &str,
        document: &str,
        milestone: Option<&str>,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let scope = self
            .scope_milestone(&mut *tx, r.project.clone(), milestone)
            .await?;
        let id = parse_id("document", document)?;
        if !tx
            .projects()
            .set_document_milestone(r.project.id, id, scope, self.clock.now())
            .await?
        {
            return Err(AppError::not_found(
                "Could not find the document in this project.",
            ));
        }
        self.emit_resources_changed(&mut *tx, ctx, r.project.id, scope)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Hängt einen abgeschlossenen Upload ohne Ziel-Issue als Datei an (Prüfungen wie bei
    /// `create_attachment_from_upload`).
    pub async fn add_resource_file(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: Option<&str>,
        asset_url: &str,
        title: Option<&str>,
    ) -> AppResult<Uuid> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let scope = self
            .scope_milestone(&mut *tx, r.project.clone(), milestone)
            .await?;
        let asset_url = asset_url.trim();
        let key = asset_url
            .strip_prefix(&self.asset_prefix())
            .ok_or_else(|| {
                upload_error(
                    "invalid_asset_url",
                    "assetUrl does not belong to this workspace",
                )
            })?;
        if key.split('/').next() != Some(ctx.org_id.0.to_string().as_str()) {
            return Err(upload_error(
                "invalid_asset_url",
                "assetUrl does not belong to this workspace",
            ));
        }
        let reservation = tx
            .attachments()
            .reservation_by_key(key)
            .await?
            .ok_or_else(|| upload_error("upload_unknown", "Upload not found."))?;
        if reservation.issue_id.is_some() {
            return Err(upload_error(
                "upload_issue_mismatch",
                "assetUrl was prepared for an issue",
            ));
        }
        let actor = resolve::user::actor_user_id(ctx);
        if reservation.creator_id != actor {
            return Err(upload_error(
                "upload_not_owned",
                "assetUrl was prepared by a different user",
            ));
        }
        let now = self.clock.now();
        let stored = self
            .blobs
            .head(key)
            .await?
            .ok_or_else(|| upload_error("upload_missing", "Upload not found."))?;
        if i64::try_from(stored.size).ok() != Some(reservation.size) {
            return Err(upload_error(
                "upload_size_mismatch",
                "The uploaded file size differs from the announced size",
            ));
        }
        if reservation.expires_at < now {
            return Err(upload_error(
                "upload_expired",
                "The upload reservation expired. Rerun the upload and try again",
            ));
        }
        // S-15: Speicher-Quota auch auf dem Projektdatei-Pfad prüfen (additional = 0: die Bytes
        // stecken bereits über die offene Reservierung in der Summe).
        crate::usecases::attachment::enforce_storage_quota(&mut *tx, &snap, 0, now).await?;
        if !tx
            .attachments()
            .finalize_reservation(reservation.id, now)
            .await?
        {
            return Err(AppError::conflict("The upload was already finalized")
                .with_details(json!({ "reason": "upload_finalized" })));
        }
        let id = self.ids.new_uuid();
        tx.projects()
            .insert_file(
                &ProjectFile {
                    id,
                    project_id: r.project.id,
                    milestone_id: scope,
                    url: asset_url.to_owned(),
                    storage_key: key.to_owned(),
                    title: title
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| reservation.filename.clone()),
                    content_type: Some(reservation.content_type.clone()),
                    size: Some(reservation.size),
                    creator_id: actor,
                    created_at: now,
                },
                now,
            )
            .await?;
        self.emit_resources_changed(&mut *tx, ctx, r.project.id, scope)
            .await?;
        // S-13/S-14: Reservierung nach dem Anlegen der Projektdatei löschen — project_file hält den
        // storage_key. Bliebe sie stehen, blockierte sie (wie bei Issue-Anhängen) dauerhaft das
        // physische Aufräumen gelöschter Dateien.
        tx.attachments()
            .delete_reservations(&[reservation.id])
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Löscht eine Datei samt Objekt im Speicher.
    pub async fn remove_resource_file(
        &self,
        ctx: &RequestCtx,
        project: &str,
        file: &str,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let id = parse_id("file", file)?;
        let key = tx
            .projects()
            .delete_file(r.project.id, id)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find the file."))?;
        if let Some(reservation) = tx.attachments().reservation_by_key(&key).await? {
            tx.attachments()
                .delete_reservations(&[reservation.id])
                .await?;
        }
        self.emit_resources_changed(&mut *tx, ctx, r.project.id, None)
            .await?;
        tx.commit().await?;
        self.delete_blobs(&[key]).await;
        Ok(())
    }

    async fn emit_resources_changed(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        project: lineal_domain::ids::ProjectId,
        milestone: Option<MilestoneId>,
    ) -> AppResult<()> {
        let event = match milestone {
            Some(milestone_id) => DomainEvent::ProjectMilestoneUpdated {
                milestone_id,
                project_id: project,
            },
            None => DomainEvent::ProjectUpdated {
                project_id: project,
                changed: vec!["resources".into()],
                previous_lead_id: None,
            },
        };
        emit(tx, ctx, event).await
    }
}

fn parse_id(field: &str, value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value.trim()).map_err(|_| {
        AppError::invalid_request(format!("{field} must be an id"))
            .with_details(json!({ "field": field }))
    })
}
