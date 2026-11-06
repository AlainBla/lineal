//! usecases – Meilensteine (docs/04 §3): anlegen, ändern, auflisten, löschen.

use super::view::{MilestoneView, milestone_views, team_tz};
use super::{ProjectView, SaveProjectInput, emit, require_manage, writable};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::repos::project::{
    MilestoneCompletion, MilestoneDateChange, MilestonePatch, NewMilestone, Project,
};
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

/// Was beim Abschließen mit offenen Issues geschieht.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum OpenIssues {
    /// Bleiben als offen am abgeschlossenen Meilenstein.
    #[default]
    Keep,
    /// Bleiben im Projekt, ohne Meilenstein.
    Remove,
    /// Wandern in einen anderen Meilenstein (Referenz).
    Move(String),
}

fn validate_name(name: &str) -> AppResult<String> {
    let n = super::clean_text(name);
    Ok(lineal_domain::rules::text::validate_single_line_name(
        "name", &n, 255,
    )?)
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
        if let Some(d) = &input.description {
            super::pure::check_len("description", d, 100_000)?;
        }
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
                    return Err(AppError::conflict(
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
                if let Some(to_date) = input.target_date {
                    if to_date != m.target_date {
                        tx.projects()
                            .add_milestone_date_change(&MilestoneDateChange {
                                milestone_id: m.id,
                                from_date: m.target_date,
                                to_date,
                                actor_id: resolve::user::actor_user_id(ctx),
                                created_at: now,
                            })
                            .await?;
                    }
                }
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

    /// Schließt einen Meilenstein bewusst ab: friert die Issue-Liste ein, behandelt offene Issues
    /// und hält die Ergebnisnotiz fest (docs/04 §3.6).
    pub async fn complete_milestone(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: &str,
        open: OpenIssues,
        note: Option<&str>,
    ) -> AppResult<(ProjectRef, MilestoneView)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, milestone).await?;
        if m.completed_at.is_some() {
            return Err(AppError::conflict("The milestone is already completed.")
                .with_details(json!({ "reason": "milestone_completed" })));
        }
        if let Some(n) = note {
            super::pure::check_len("note", n, 100_000)?;
        }
        let note = note
            .map(lineal_domain::rules::markdown::sanitize)
            .filter(|n| !n.trim().is_empty());
        let now = self.clock.now();
        let actor = resolve::user::actor_user_id(ctx);
        let refs = tx.projects().milestone_issue_refs(m.id).await?;
        let visible = snap.visible_team_ids();
        let target = match &open {
            OpenIssues::Move(reference) => {
                let t =
                    resolve::project::resolve_milestone(&mut *tx, r.project.id, reference).await?;
                if t.id == m.id || t.completed_at.is_some() {
                    return Err(AppError::invalid_request(
                        "Open issues can only move to another open milestone.",
                    )
                    .with_details(json!({ "field": "moveTo" })));
                }
                Some(t)
            }
            _ => None,
        };
        let snapshot = json!(
            refs.iter()
                .map(|i| json!({
                    "id": i.id.to_string(), "teamId": i.team_id.to_string(),
                    "identifier": i.identifier, "title": i.title,
                    "stateName": i.state_name, "stateType": i.state_type,
                }))
                .collect::<Vec<_>>()
        );
        if !matches!(open, OpenIssues::Keep) {
            let moved = tx
                .projects()
                .move_open_milestone_issues(m.id, target.as_ref().map(|t| t.id), actor, now)
                .await?;
            // Issues unsichtbarer Teams dürfen nicht mitverändert werden.
            if moved.iter().any(|(_, t)| !visible.contains(t)) {
                return Err(AppError::forbidden(
                    "The milestone is used by issues of teams you cannot access.",
                )
                .with_details(json!({ "reason": "milestone_in_hidden_team" })));
            }
            for (issue, team) in moved {
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
        }
        tx.projects()
            .set_milestone_completion(
                m.id,
                Some(MilestoneCompletion {
                    by: actor,
                    note,
                    snapshot,
                }),
                now,
            )
            .await?;
        self.finish_milestone_change(tx, ctx, &snap, &r.project, m.id, now)
            .await
    }

    /// Eingefrorene Issue-Liste, gefiltert auf die für den Aufrufer sichtbaren Teams.
    pub async fn milestone_snapshot(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: &str,
    ) -> AppResult<Vec<serde_json::Value>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, milestone).await?;
        let visible: Vec<String> = snap
            .visible_team_ids()
            .iter()
            .map(|t| t.to_string())
            .collect();
        Ok(m.snapshot
            .and_then(|s| s.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|i| {
                i["teamId"]
                    .as_str()
                    .is_some_and(|t| visible.iter().any(|v| v == t))
            })
            .collect())
    }

    /// Öffnet einen abgeschlossenen Meilenstein wieder; die Ergebnisnotiz bleibt.
    pub async fn reopen_milestone(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: &str,
    ) -> AppResult<(ProjectRef, MilestoneView)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        writable(&r.project)?;
        let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, milestone).await?;
        let now = self.clock.now();
        tx.projects()
            .set_milestone_completion(m.id, None, now)
            .await?;
        self.finish_milestone_change(tx, ctx, &snap, &r.project, m.id, now)
            .await
    }

    async fn finish_milestone_change(
        &self,
        mut tx: Box<dyn crate::ports::Tx>,
        ctx: &RequestCtx,
        snap: &PolicySnapshot,
        project: &Project,
        id: MilestoneId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<(ProjectRef, MilestoneView)> {
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectMilestoneUpdated {
                milestone_id: id,
                project_id: project.id,
            },
        )
        .await?;
        let views = milestone_views(
            &mut *tx,
            project.id,
            team_tz(snap, project.lead_team_id),
            now,
        )
        .await?;
        let view = views
            .into_iter()
            .find(|v| v.milestone.id == id)
            .ok_or_else(|| AppError::internal("milestone view missing"))?;
        let out = (ProjectRef::of(snap, project), view);
        tx.commit().await?;
        Ok(out)
    }

    /// Wandelt einen offenen Meilenstein in ein Projekt um: neues Projekt mit Name, Beschreibung,
    /// Teams und Priorität, Issues und Ressourcen wandern mit, der Meilenstein entfällt
    /// (docs/04 §3.4).
    pub async fn convert_milestone_to_project(
        &self,
        ctx: &RequestCtx,
        project: &str,
        milestone: &str,
    ) -> AppResult<ProjectView> {
        let (source, m, teams) = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
            let r = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
            require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
            writable(&r.project)?;
            let m = resolve::project::resolve_milestone(&mut *tx, r.project.id, milestone).await?;
            if m.completed_at.is_some() {
                return Err(AppError::invalid_request(
                    "A completed milestone cannot become a project.",
                )
                .with_details(json!({ "reason": "milestone_completed" })));
            }
            let visible = snap.visible_team_ids();
            let refs = tx.projects().milestone_issue_refs(m.id).await?;
            if refs.iter().any(|i| !visible.contains(&i.team_id)) {
                return Err(AppError::forbidden(
                    "The milestone is used by issues of teams you cannot access.",
                )
                .with_details(json!({ "reason": "milestone_in_hidden_team" })));
            }
            // Nur sichtbare Teams übernehmen: ein privates Team kann die Person nicht zuordnen.
            let teams: Vec<_> = r
                .teams
                .iter()
                .copied()
                .filter(|t| visible.contains(t))
                .collect();
            (r.project, m, teams)
        };
        let created = self
            .save_project(
                ctx,
                SaveProjectInput {
                    name: Some(m.name.clone()),
                    description: m.description.clone(),
                    priority: Some(source.priority),
                    lead_team: Some(source.lead_team_id.to_string()),
                    add_teams: teams.iter().map(ToString::to_string).collect(),
                    ..Default::default()
                },
            )
            .await?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let now = self.clock.now();
        let actor = resolve::user::actor_user_id(ctx);
        let moved = tx
            .projects()
            .move_milestone_to_project(m.id, source.id, created.project.id, actor, now)
            .await?;
        for (issue, team) in moved {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::IssueUpdated {
                    previous: None,
                    issue_id: issue,
                    team_id: team,
                    changed: vec!["projectId".into(), "projectMilestoneId".into()],
                    mentioned_user_ids: vec![],
                },
            )
            .await?;
        }
        tx.projects().archive_milestone(m.id, actor, now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectMilestoneRemoved {
                milestone_id: m.id,
                project_id: source.id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(created)
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
        // Issues unsichtbarer Teams dürfen nicht mitverändert werden.
        let visible = snap.visible_team_ids();
        if affected.iter().any(|(_, t)| !visible.contains(t)) {
            return Err(AppError::forbidden(
                "The milestone is used by issues of teams you cannot access.",
            )
            .with_details(json!({ "reason": "milestone_in_hidden_team" })));
        }
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
