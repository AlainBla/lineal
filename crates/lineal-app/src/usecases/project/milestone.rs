//! usecases – Meilensteine (docs/04 §3): anlegen, ändern, auflisten, löschen.

use super::view::{MilestoneView, milestone_views, team_tz};
use super::{emit, require_manage, writable};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::repos::project::{MilestonePatch, NewMilestone, Project};
use crate::resolve;
use chrono::NaiveDate;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{MilestoneId, ProjectId};
use serde_json::json;

/// Eingebettete Projekt-Referenz (`ProjectRef`): UUID, Kennung, Name.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRef {
    pub id: ProjectId,
    pub identifier: String,
    pub name: String,
}

impl ProjectRef {
    pub(crate) fn of(snap: &PolicySnapshot, p: &Project) -> Self {
        let key = snap
            .tree
            .get(p.lead_team_id)
            .map(|t| t.key.clone())
            .unwrap_or_default();
        Self {
            id: p.id,
            identifier: format!("P-{}-{}", key, p.number),
            name: p.name.clone(),
        }
    }
}

/// Eingabe von `save_milestone`.
#[derive(Debug, Clone, Default)]
pub struct SaveMilestoneInput {
    /// Projekt-Referenz (immer Pflicht, auch beim Update).
    pub project: String,
    /// Meilenstein-Referenz (UUID oder Name) für das Update.
    pub id: Option<String>,
    pub name: Option<String>,
    /// Markdown; leerer String leert.
    pub description: Option<String>,
    /// `Some(None)` leert das Zieldatum.
    pub target_date: Option<Option<NaiveDate>>,
}

fn validate_name(name: &str) -> AppResult<String> {
    let n = super::clean_text(name);
    let n = n.as_str();
    if n.is_empty() || n.chars().count() > 255 {
        return Err(AppError::invalid_request("name must be 1-255 characters")
            .with_details(json!({ "field": "name" })));
    }
    Ok(n.to_owned())
}

impl App {
    /// `list_milestones`: alle Meilensteine in `sortOrder`-Reihenfolge.
    pub async fn list_milestones(
        &self,
        ctx: &RequestCtx,
        project: &str,
    ) -> AppResult<(ProjectRef, Vec<MilestoneView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        let views = milestone_views(
            &mut *tx,
            r.project.id,
            team_tz(&snap, r.project.lead_team_id),
            self.clock.now(),
        )
        .await?;
        Ok((ProjectRef::of(&snap, &r.project), views))
    }

    /// `get_milestone`: ein Meilenstein per UUID oder Name innerhalb des Projekts.
    pub async fn get_milestone(
        &self,
        ctx: &RequestCtx,
        project: &str,
        query: &str,
    ) -> AppResult<(ProjectRef, MilestoneView)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, query).await?;
        let views = milestone_views(
            &mut *tx,
            r.project.id,
            team_tz(&snap, r.project.lead_team_id),
            self.clock.now(),
        )
        .await?;
        let view = views
            .into_iter()
            .find(|v| v.milestone.id == m.id)
            .ok_or_else(|| AppError::internal("milestone view missing"))?;
        Ok((ProjectRef::of(&snap, &r.project), view))
    }

    /// `save_milestone`: legt an (ohne `id`; ans Ende, `sortOrder = max + 1`) oder ändert.
    pub async fn save_milestone(
        &self,
        ctx: &RequestCtx,
        input: SaveMilestoneInput,
    ) -> AppResult<(ProjectRef, MilestoneView)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, &input.project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let now = self.clock.now();
        let description = input
            .description
            .as_deref()
            .map(lineal_domain::rules::markdown::sanitize)
            .map(|d| (!d.is_empty()).then_some(d));
        let id = match &input.id {
            None => {
                let name = validate_name(input.name.as_deref().unwrap_or(""))?;
                if tx
                    .projects()
                    .find_milestone_by_name(r.project.id, &name)
                    .await?
                    .is_some()
                {
                    return Err(AppError::invalid_request(
                        "A milestone with this name already exists in the project.",
                    )
                    .with_details(json!({ "field": "name" })));
                }
                let sort_order = tx.projects().max_milestone_sort_order(r.project.id).await? + 1.0;
                let id = MilestoneId(self.ids.new_uuid());
                tx.projects()
                    .insert_milestone(&NewMilestone {
                        id,
                        project_id: r.project.id,
                        name,
                        description: description.flatten(),
                        target_date: input.target_date.flatten(),
                        sort_order,
                        now,
                    })
                    .await?;
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::ProjectMilestoneCreated {
                        milestone_id: id,
                        project_id: r.project.id,
                    },
                )
                .await?;
                id
            }
            Some(reference) => {
                let m =
                    resolve::project::resolve_milestone(&mut *tx, r.project.id, reference).await?;
                let name = input.name.as_deref().map(validate_name).transpose()?;
                tx.projects()
                    .update_milestone(
                        m.id,
                        &MilestonePatch {
                            name,
                            description,
                            target_date: input.target_date,
                            sort_order: None,
                        },
                        now,
                    )
                    .await?;
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::ProjectMilestoneUpdated {
                        milestone_id: m.id,
                        project_id: r.project.id,
                    },
                )
                .await?;
                m.id
            }
        };
        let views = milestone_views(
            &mut *tx,
            r.project.id,
            team_tz(&snap, r.project.lead_team_id),
            now,
        )
        .await?;
        let view = views
            .into_iter()
            .find(|v| v.milestone.id == id)
            .ok_or_else(|| AppError::internal("milestone view missing"))?;
        let out = (ProjectRef::of(&snap, &r.project), view);
        tx.commit().await?;
        Ok(out)
    }

    /// Löscht einen Meilenstein: Issues verlieren die Zuordnung (History-Eintrag je Issue), der
    /// Meilenstein wird archiviert (docs/04 §3.3).
    pub async fn delete_milestone(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: &str,
    ) -> AppResult<MilestoneId> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, milestone).await?;
        let now = self.clock.now();
        let affected = tx
            .projects()
            .archive_milestone(m.id, resolve::user::actor_user_id(ctx), now)
            .await?;
        for (issue, team) in affected {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::IssueUpdated {
                    previous: None,
                    issue_id: issue,
                    team_id: team,
                    changed: vec!["projectMilestoneId".into()],
                    mentioned_user_ids: vec![],
                },
            )
            .await?;
        }
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectMilestoneRemoved {
                milestone_id: m.id,
                project_id: r.project.id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(m.id)
    }
}
