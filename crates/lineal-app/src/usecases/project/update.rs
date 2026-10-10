//! usecases – Status-Updates von Projekten (docs/04 §5): veröffentlichen, bearbeiten, löschen
//! (= archivieren), Health des Projekts, Diff zum vorherigen Update, Staleness.

use super::milestone::ProjectRef;
use super::pure::{Staleness, compute_diff, staleness};
use super::view::{milestone_views, team_tz};
use super::{emit, require_manage, writable};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::project::{Project, ProjectPatch, ProjectUpdate, ProjectUpdateFilter};
use crate::ports::repos::user::{Page, PageRequest, User};
use crate::resolve;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{Health, ProjectStatusType};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{ProjectId, ProjectUpdateId};
use serde_json::{Value, json};
use uuid::Uuid;

/// Ein Status-Update mit abgeleiteten Feldern.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectUpdateView {
    pub update: ProjectUpdate,
    pub project: ProjectRef,
    /// Slug des Projekts (für URLs).
    pub project_slug: String,
    pub user: Option<User>,
    /// Das letzte Update eines Projekts im Zustand „Update missing“ oder grau.
    pub is_stale: bool,
    pub comment_count: i64,
}

/// Filter von `get_status_updates` (Projekt).
#[derive(Debug, Clone, Default)]
pub struct ListProjectUpdatesInput {
    pub project: Option<String>,
    pub user: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub page: PageRequest,
}

/// Länge des Erinnerungszyklus in Tagen (docs/04 §5.4): `never` = keine Erwartung.
pub(crate) fn reminder_cycle_days(mode: &str, own_weeks: Option<i32>, org_weeks: Option<i32>) -> Option<i64> {
    let weeks = match mode {
        "never" => None,
        "custom" => own_weeks.or(org_weeks),
        _ => org_weeks,
    };
    weeks.filter(|w| *w > 0).map(|w| i64::from(w) * 7)
}

/// Interner Snapshot des Projekts (Basis des Diffs, docs/04 §5.3).
pub(crate) async fn project_snapshot(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    p: &Project,
    now: DateTime<Utc>,
) -> AppResult<Value> {
    let statuses = tx.projects().list_statuses().await?;
    let status = statuses.iter().find(|s| s.id == p.status_id);
    let rows = tx.projects().issue_rows(p.id).await?;
    let scope = super::view::ProjectScope::from_rows(&rows);
    let milestones = milestone_views(&mut *tx, p.id, team_tz(snap, p.lead_team_id), now).await?;
    let lead_name = match p.lead_id {
        Some(id) => tx.users().get(id).await?.map(|u| u.name),
        None => None,
    };
    Ok(json!({
        "status": status.map(|s| json!({ "id": s.id, "name": s.name, "type": s.status_type.as_str() })),
        "targetDate": p.target_date.map(|d| d.to_string()),
        "targetDateResolution": p.target_date_resolution.map(|r| r.as_str()),
        "leadId": p.lead_id.map(|u| u.to_string()),
        "leadName": lead_name,
        "progress": scope.progress,
        "milestones": milestones.iter().map(|m| json!({
            "id": m.milestone.id.to_string(), "name": m.milestone.name, "progress": m.progress,
        })).collect::<Vec<_>>(),
    }))
}

async fn view_of(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    u: ProjectUpdate,
    now: DateTime<Utc>,
) -> AppResult<ProjectUpdateView> {
    let project = tx
        .projects()
        .get(u.project_id)
        .await?
        .ok_or_else(|| AppError::internal("project of update missing"))?;
    let statuses = tx.projects().list_statuses().await?;
    let status = statuses.iter().find(|s| s.id == project.status_id);
    let settings = tx.projects().reminder_settings().await?;
    let latest = tx.projects().latest_update(project.id, None).await?;
    let expected = status.is_some_and(|s| s.status_type == ProjectStatusType::Started && !s.indefinite);
    let days = reminder_cycle_days(
        &project.update_reminder_mode,
        project.update_reminder_frequency_in_weeks,
        settings.and_then(|s| s.project_frequency_in_weeks),
    );
    let st = staleness(
        expected,
        days,
        latest.as_ref().map(|l| (l.created_at, l.health)),
        now,
    );
    let is_stale = latest.as_ref().is_some_and(|l| l.id == u.id)
        && matches!(st, Staleness::Missing | Staleness::Grey);
    let user = tx.users().get(u.user_id).await?;
    let comment_count = tx.projects().update_comment_count(u.id).await?;
    Ok(ProjectUpdateView {
        project: ProjectRef::of(snap, &project),
        project_slug: project.slug_id.clone(),
        user,
        is_stale,
        comment_count,
        update: u,
    })
}

/// Setzt `lastUpdate`, `health` und `healthUpdatedAt` des Projekts aus dem jüngsten nicht
/// archivierten Update (oder leert sie).
async fn refresh_parent(
    tx: &mut dyn Tx,
    project: ProjectId,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let latest = tx.projects().latest_update(project, None).await?;
    let patch = match latest {
        Some(l) => ProjectPatch {
            last_update_id: Some(Some(l.id)),
            health: Some(Some(l.health)),
            health_updated_at: Some(Some(l.created_at)),
            ..Default::default()
        },
        None => ProjectPatch {
            last_update_id: Some(None),
            health: Some(None),
            health_updated_at: Some(None),
            ..Default::default()
        },
    };
    tx.projects().update(project, &patch, now).await?;
    Ok(())
}

async fn resolve_update(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
) -> AppResult<(ProjectUpdate, Vec<lineal_domain::ids::TeamId>)> {
    let nf = || {
        AppError::not_found(format!("Could not find status update \"{reference}\""))
            .with_details(json!({ "reference": reference, "type": "ProjectUpdate" }))
    };
    let id = Uuid::parse_str(reference.trim()).map_err(|_| nf())?;
    let u = tx
        .projects()
        .get_update(ProjectUpdateId(id))
        .await?
        .ok_or_else(nf)?;
    let teams = tx.projects().list_teams(u.project_id).await?;
    let visible = snap.visible_team_ids();
    if !teams.iter().any(|t| visible.contains(t)) {
        return Err(nf());
    }
    Ok((u, teams))
}

impl App {
    /// `get_status_updates` (Projekt): Liste mit Filtern.
    pub async fn list_project_updates(
        &self,
        ctx: &RequestCtx,
        input: ListProjectUpdatesInput,
    ) -> AppResult<(Page<ProjectUpdate>, Vec<ProjectUpdateView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let mut filter = ProjectUpdateFilter {
            visible_teams: snap.visible_team_ids().into_iter().collect(),
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            ..Default::default()
        };
        if let Some(p) = &input.project {
            filter.project = Some(resolve::project::resolve_in(&mut *tx, &snap, p).await?.project.id);
        }
        if let Some(u) = &input.user {
            filter.user = Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, u).await?.id);
        }
        let page = tx.projects().list_updates(&filter, &input.page).await?;
        let mut views = Vec::with_capacity(page.items.len());
        for u in page.items.clone() {
            views.push(view_of(&mut *tx, &snap, u, now).await?);
        }
        Ok((page, views))
    }

    /// Ein Status-Update (UUID).
    pub async fn get_project_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<ProjectUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (u, _) = resolve_update(&mut *tx, &snap, reference).await?;
        view_of(&mut *tx, &snap, u, self.clock.now()).await
    }

    /// `save_status_update` (Create, Projekt): erstes Update nur durch den Lead; `health`
    /// Default = Health des vorherigen Updates, sonst `onTrack`.
    pub async fn post_project_update(
        &self,
        ctx: &RequestCtx,
        project: &str,
        body: &str,
        health: Option<Health>,
        is_diff_hidden: bool,
    ) -> AppResult<ProjectUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::PostProjectUpdate, &r.teams)?;
        writable(&r.project)?;
        let body = lineal_domain::rules::markdown::sanitize(body.trim());
        if body.is_empty() {
            return Err(AppError::invalid_request("`body` must not be empty")
                .with_details(json!({ "field": "body" })));
        }
        let actor = resolve::user::actor_user_id(ctx)
            .ok_or_else(|| AppError::forbidden("Status updates need a user."))?;
        let previous = tx.projects().latest_update(r.project.id, None).await?;
        if previous.is_none()
            && let Some(lead) = r.project.lead_id
            && lead != actor
            && !snap.actor.is_system()
        {
            return Err(AppError::forbidden(
                "Only the project lead can publish the first status update.",
            )
            .with_details(json!({ "reason": "first_update_lead_only" })));
        }
        let health = health
            .or(previous.as_ref().map(|p| p.health))
            .unwrap_or(Health::OnTrack);
        let snapshot = project_snapshot(&mut *tx, &snap, &r.project, now).await?;
        let (diff, diff_markdown) = match &previous {
            Some(p) => match compute_diff(&p.snapshot, &snapshot, true) {
                Some((d, md)) => (Some(d), Some(md)),
                None => (None, None),
            },
            None => (None, None),
        };
        let id = ProjectUpdateId(self.ids.new_uuid());
        tx.projects()
            .insert_update(&crate::ports::repos::project::NewProjectUpdate {
                id,
                project_id: r.project.id,
                user_id: actor,
                body,
                health,
                snapshot,
                diff,
                diff_markdown,
                is_diff_hidden,
                now,
            })
            .await?;
        refresh_parent(&mut *tx, r.project.id, now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectUpdateCreated {
                update_id: id,
                project_id: r.project.id,
            },
        )
        .await?;
        let u = tx
            .projects()
            .get_update(id)
            .await?
            .ok_or_else(|| AppError::internal("update missing"))?;
        let view = view_of(&mut *tx, &snap, u, now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// `save_status_update` (Update): nur der Autor; setzt `editedAt`, der Diff bleibt.
    pub async fn edit_project_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        body: Option<&str>,
        health: Option<Health>,
    ) -> AppResult<ProjectUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (u, teams) = resolve_update(&mut *tx, &snap, reference).await?;
        require_manage(&snap, ctx, Action::PostProjectUpdate, &teams)?;
        if resolve::user::actor_user_id(ctx) != Some(u.user_id) {
            return Err(AppError::forbidden("Only the author can edit a status update.")
                .with_details(json!({ "reason": "not_author" })));
        }
        if u.archived_at.is_some() {
            return Err(AppError::invalid_request("The status update is archived.")
                .with_details(json!({ "reason": "update_archived" })));
        }
        let clean = body.map(|b| lineal_domain::rules::markdown::sanitize(b.trim()));
        if clean.as_deref().is_some_and(str::is_empty) {
            return Err(AppError::invalid_request("`body` must not be empty")
                .with_details(json!({ "field": "body" })));
        }
        let edited = tx
            .projects()
            .edit_update(u.id, clean.as_deref(), health, now)
            .await?;
        if health.is_some() {
            refresh_parent(&mut *tx, u.project_id, now).await?;
        }
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectUpdateUpdated {
                update_id: u.id,
                project_id: u.project_id,
            },
        )
        .await?;
        let view = view_of(&mut *tx, &snap, edited, now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// `delete_status_update`: archiviert; `health` des Projekts fällt auf das nächstjüngere zurück.
    pub async fn delete_project_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<(ProjectUpdateId, DateTime<Utc>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (u, teams) = resolve_update(&mut *tx, &snap, reference).await?;
        require_manage(&snap, ctx, Action::PostProjectUpdate, &teams)?;
        let is_author = resolve::user::actor_user_id(ctx) == Some(u.user_id);
        if !(is_author || snap.actor.is_admin() || snap.actor.is_system()) {
            return Err(
                AppError::forbidden("Only the author or an admin can delete a status update.")
                    .with_details(json!({ "reason": "not_author" })),
            );
        }
        if u.archived_at.is_some() {
            return Err(AppError::not_found("Could not find status update.")
                .with_details(json!({ "reference": reference, "type": "ProjectUpdate" })));
        }
        tx.projects().archive_update(u.id, now).await?;
        refresh_parent(&mut *tx, u.project_id, now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectUpdateRemoved {
                update_id: u.id,
                project_id: u.project_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok((u.id, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_days_follow_mode() {
        assert_eq!(reminder_cycle_days("followWorkspace", Some(2), Some(1)), Some(7));
        assert_eq!(reminder_cycle_days("custom", Some(2), Some(1)), Some(14));
        assert_eq!(reminder_cycle_days("custom", None, Some(1)), Some(7));
        assert_eq!(reminder_cycle_days("never", Some(2), Some(1)), None);
        assert_eq!(reminder_cycle_days("followWorkspace", None, None), None);
    }
}
