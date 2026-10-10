//! GraphQL-Teilobjekte des Bereichs `project` (WP07): Projekte, Meilensteine, Projekt-Updates.
//!
//! Dieselben Use-Cases wie MCP: Rechte (alle Teams), Projekt-Hülle (private Teams nur für Mitglieder)
//! gelten unverändert. `progress` ist 0..1.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use chrono::NaiveDate;
use lineal_app::AppError;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::project::milestone::{ProjectRef, SaveMilestoneInput};
use lineal_app::usecases::project::pure::parse_date;
use lineal_app::usecases::project::update::{ListProjectUpdatesInput, ProjectUpdateView};
use lineal_app::usecases::project::{
    ListProjectsInput, MilestoneView, ProjectView, SaveProjectInput, ViewOpts,
};
use lineal_domain::enums::{DateResolution, Health};

#[derive(SimpleObject)]
#[graphql(name = "ProjectReference")]
pub struct ProjectRefGql {
    pub id: String,
    pub identifier: String,
    pub name: String,
}

impl From<ProjectRef> for ProjectRefGql {
    fn from(p: ProjectRef) -> Self {
        Self {
            id: p.id.to_string(),
            identifier: p.identifier,
            name: p.name,
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "ProjectStatusInfo")]
pub struct ProjectStatusGql {
    pub id: String,
    pub name: String,
    #[graphql(name = "type")]
    pub status_type: String,
    pub color: String,
}

#[derive(SimpleObject)]
#[graphql(name = "PlanningRef")]
pub struct SimpleRefGql {
    pub id: String,
    pub name: String,
    pub key: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "Project")]
pub struct ProjectGql {
    pub id: String,
    pub identifier: String,
    pub previous_identifiers: Vec<String>,
    pub slug_id: String,
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: String,
    pub priority: i32,
    pub status: ProjectStatusGql,
    pub health: Option<String>,
    /// 0..1.
    pub progress: f64,
    pub scope: f64,
    pub completed_scope: f64,
    pub issue_count: i64,
    pub completed_issue_count: i64,
    pub start_date: Option<String>,
    pub start_date_resolution: Option<String>,
    pub target_date: Option<String>,
    pub target_date_resolution: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub canceled_at: Option<String>,
    pub archived_at: Option<String>,
    /// `null`, wenn das Lead-Team für den Betrachter nicht zugänglich ist.
    pub lead_team: Option<SimpleRefGql>,
    /// Nur für den Betrachter zugängliche Teams (Projekt-Hülle).
    pub teams: Vec<SimpleRefGql>,
    pub lead: Option<SimpleRefGql>,
    pub members: Vec<SimpleRefGql>,
    pub labels: Vec<SimpleRefGql>,
    pub initiatives: Vec<SimpleRefGql>,
    pub milestones: Vec<MilestoneGql>,
    /// Gesetzt, wenn der Status in `completed`/`canceled` wechselt: Anzahl offener Issues.
    pub open_issue_count: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

fn user_ref(u: &lineal_app::ports::repos::user::User) -> SimpleRefGql {
    SimpleRefGql {
        id: u.id.to_string(),
        name: u.name.clone(),
        key: None,
    }
}

impl From<ProjectView> for ProjectGql {
    fn from(v: ProjectView) -> Self {
        let pref = ProjectRef {
            id: v.project.id,
            identifier: v.identifier.clone(),
            name: v.project.name.clone(),
        };
        let p = v.project;
        Self {
            id: p.id.to_string(),
            identifier: v.identifier,
            previous_identifiers: v.previous_identifiers,
            slug_id: p.slug_id,
            name: p.name,
            summary: p.description,
            description: p.content,
            icon: p.icon,
            color: p.color,
            priority: p.priority,
            status: ProjectStatusGql {
                id: v.status.id.to_string(),
                name: v.status.name,
                status_type: v.status.status_type.as_str().into(),
                color: v.status.color,
            },
            health: p.health.map(|h| h.as_str().into()),
            progress: v.scope.progress,
            scope: v.scope.scope,
            completed_scope: v.scope.completed_scope,
            issue_count: v.scope.issue_count,
            completed_issue_count: v.scope.completed_issue_count,
            start_date: p.start_date.map(|d| d.to_string()),
            start_date_resolution: p.start_date_resolution.map(|r| r.as_str().into()),
            target_date: p.target_date.map(|d| d.to_string()),
            target_date_resolution: p.target_date_resolution.map(|r| r.as_str().into()),
            started_at: p.started_at.map(|d| d.to_rfc3339()),
            completed_at: p.completed_at.map(|d| d.to_rfc3339()),
            canceled_at: p.canceled_at.map(|d| d.to_rfc3339()),
            archived_at: p.archived_at.map(|d| d.to_rfc3339()),
            lead_team: v.lead_team.map(|t| SimpleRefGql {
                id: t.id.to_string(),
                name: t.name,
                key: Some(t.key),
            }),
            teams: v
                .teams
                .into_iter()
                .map(|t| SimpleRefGql {
                    id: t.id.to_string(),
                    name: t.name,
                    key: Some(t.key),
                })
                .collect(),
            lead: v.lead.as_ref().map(user_ref),
            members: v.members.iter().map(user_ref).collect(),
            labels: v
                .labels
                .into_iter()
                .map(|l| SimpleRefGql {
                    id: l.id.to_string(),
                    name: l.name,
                    key: None,
                })
                .collect(),
            initiatives: v
                .initiatives
                .into_iter()
                .map(|i| SimpleRefGql {
                    id: i.id.to_string(),
                    key: Some(i.identifier()),
                    name: i.name,
                })
                .collect(),
            milestones: v
                .milestones
                .into_iter()
                .map(|m| MilestoneGql::of(&pref, m))
                .collect(),
            open_issue_count: v.open_issue_count,
            created_at: p.created_at.to_rfc3339(),
            updated_at: p.updated_at.to_rfc3339(),
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "ProjectMilestone")]
pub struct MilestoneGql {
    pub id: String,
    pub project: ProjectRefGql,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub sort_order: f64,
    /// `unstarted` | `next` | `overdue` | `done`.
    pub status: String,
    /// 0..1.
    pub progress: f64,
    pub issue_count: i64,
    pub completed_issue_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl MilestoneGql {
    fn of(p: &ProjectRef, m: MilestoneView) -> Self {
        Self {
            id: m.milestone.id.to_string(),
            project: p.clone().into(),
            name: m.milestone.name,
            description: m.milestone.description,
            target_date: m.milestone.target_date.map(|d| d.to_string()),
            sort_order: m.milestone.sort_order,
            status: m.status.as_str().into(),
            progress: m.progress,
            issue_count: m.issue_count,
            completed_issue_count: m.completed_issue_count,
            created_at: m.milestone.created_at.to_rfc3339(),
            updated_at: m.milestone.updated_at.to_rfc3339(),
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "ProjectUpdate")]
pub struct ProjectUpdateGql {
    pub id: String,
    pub project: ProjectRefGql,
    pub user: Option<SimpleRefGql>,
    pub body: String,
    pub health: String,
    pub diff: Option<String>,
    pub diff_markdown: Option<String>,
    pub is_diff_hidden: bool,
    pub is_stale: bool,
    pub comment_count: i64,
    pub edited_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ProjectUpdateView> for ProjectUpdateGql {
    fn from(v: ProjectUpdateView) -> Self {
        let u = v.update;
        Self {
            id: u.id.to_string(),
            project: v.project.into(),
            user: v.user.as_ref().map(user_ref),
            body: u.body,
            health: u.health.as_str().into(),
            diff: u.diff.map(|d| d.to_string()),
            diff_markdown: if u.is_diff_hidden {
                None
            } else {
                u.diff_markdown
            },
            is_diff_hidden: u.is_diff_hidden,
            is_stale: v.is_stale,
            comment_count: v.comment_count,
            edited_at: u.edited_at.map(|d| d.to_rfc3339()),
            archived_at: u.archived_at.map(|d| d.to_rfc3339()),
            created_at: u.created_at.to_rfc3339(),
            updated_at: u.updated_at.to_rfc3339(),
        }
    }
}

/// Eingabe für `projectCreate`/`projectUpdate`. Nicht gesetzte Felder bleiben unverändert;
/// `null` leert (wo erlaubt).
#[derive(InputObject, Default)]
pub struct ProjectInput {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub priority: Option<i32>,
    /// Projektstatus (UUID, Name oder Kategorie) – gegen den Satz des Lead-Teams aufgelöst.
    pub state: Option<String>,
    pub lead: MaybeUndefined<String>,
    pub lead_team: Option<String>,
    pub set_teams: Option<Vec<String>>,
    pub add_teams: Option<Vec<String>>,
    pub remove_teams: Option<Vec<String>>,
    /// Beim Entfernen eines Teams dessen Issues vom Projekt lösen (sonst `invalid_request`).
    pub detach_issues_of_removed_teams: Option<bool>,
    pub labels: Option<Vec<String>>,
    pub set_initiatives: Option<Vec<String>>,
    pub add_initiatives: Option<Vec<String>>,
    pub remove_initiatives: Option<Vec<String>>,
    pub start_date: MaybeUndefined<String>,
    pub start_date_resolution: MaybeUndefined<String>,
    pub target_date: MaybeUndefined<String>,
    pub target_date_resolution: MaybeUndefined<String>,
    pub links: Option<Vec<LinkInput>>,
}

#[derive(InputObject)]
pub struct LinkInput {
    pub url: String,
    pub title: String,
}

fn date(
    field: &str,
    v: MaybeUndefined<String>,
) -> async_graphql::Result<Option<Option<NaiveDate>>> {
    match v {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(s) => parse_date(field, &s)
            .map(|d| Some(Some(d)))
            .map_err(gql_err),
    }
}

fn resolution(
    field: &str,
    v: MaybeUndefined<String>,
) -> async_graphql::Result<Option<Option<DateResolution>>> {
    match v {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(s) => {
            s.parse::<DateResolution>()
                .map(|r| Some(Some(r)))
                .map_err(|_| {
                    gql_err(
                        AppError::invalid_request(format!("`{field}` is not a valid resolution"))
                            .with_details(serde_json::json!({ "field": field })),
                    )
                })
        }
    }
}

fn project_input(id: Option<String>, i: ProjectInput) -> async_graphql::Result<SaveProjectInput> {
    Ok(SaveProjectInput {
        id,
        name: i.name,
        summary: i.summary,
        description: i.description,
        patch: None,
        icon: i.icon,
        color: i.color,
        priority: i.priority,
        state: i.state,
        lead: match i.lead {
            MaybeUndefined::Undefined => None,
            MaybeUndefined::Null => Some(None),
            MaybeUndefined::Value(v) => Some(Some(v)),
        },
        lead_team: i.lead_team,
        set_teams: i.set_teams,
        add_teams: i.add_teams.unwrap_or_default(),
        remove_teams: i.remove_teams.unwrap_or_default(),
        labels: i.labels,
        set_initiatives: i.set_initiatives,
        add_initiatives: i.add_initiatives.unwrap_or_default(),
        remove_initiatives: i.remove_initiatives.unwrap_or_default(),
        links: i
            .links
            .unwrap_or_default()
            .into_iter()
            .map(|l| (l.url, l.title))
            .collect(),
        start_date: date("startDate", i.start_date)?,
        start_date_resolution: resolution("startDateResolution", i.start_date_resolution)?,
        target_date: date("targetDate", i.target_date)?,
        target_date_resolution: resolution("targetDateResolution", i.target_date_resolution)?,
        template: None,
        detach_issues_of_removed_teams: i.detach_issues_of_removed_teams.unwrap_or(false),
    })
}

#[derive(InputObject)]
pub struct MilestoneInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target_date: MaybeUndefined<String>,
}

#[derive(SimpleObject)]
pub struct ProjectPayload {
    pub success: bool,
    pub project: Option<ProjectGql>,
}

#[derive(SimpleObject)]
pub struct ProjectMilestonePayload {
    pub success: bool,
    pub project_milestone: Option<MilestoneGql>,
}

#[derive(SimpleObject)]
pub struct ProjectUpdatePayload {
    pub success: bool,
    pub project_update: Option<ProjectUpdateGql>,
}

#[derive(SimpleObject)]
pub struct ProjectSuccessPayload {
    pub success: bool,
}

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// Sichtbare Projekte (mindestens ein zugängliches Team).
    #[allow(clippy::too_many_arguments)]
    async fn projects(
        &self,
        ctx: &Context<'_>,
        team: Option<String>,
        include_sub_teams: Option<bool>,
        initiative: Option<String>,
        member: Option<String>,
        state: Option<String>,
        label: Option<String>,
        query: Option<String>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<ProjectGql>> {
        let (_, views) = app(ctx)?
            .list_projects(
                request_ctx(ctx)?,
                ListProjectsInput {
                    team,
                    include_sub_teams: include_sub_teams.unwrap_or(false),
                    initiative,
                    member,
                    state,
                    label,
                    query,
                    include_archived: include_archived.unwrap_or(false),
                    include_members: true,
                    include_milestones: true,
                    page: PageRequest {
                        limit: i64::from(first.unwrap_or(50).clamp(1, 50)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(views.into_iter().map(Into::into).collect())
    }

    /// Ein Projekt per UUID, Kennung (`P-ENG-7`), Slug oder Name.
    async fn project(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<ProjectGql> {
        Ok(app(ctx)?
            .get_project(
                request_ctx(ctx)?,
                &id,
                ViewOpts {
                    members: true,
                    milestones: true,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Meilensteine eines Projekts in `sortOrder`-Reihenfolge.
    async fn project_milestones(
        &self,
        ctx: &Context<'_>,
        project_id: String,
    ) -> async_graphql::Result<Vec<MilestoneGql>> {
        let (p, list) = app(ctx)?
            .list_milestones(request_ctx(ctx)?, &project_id)
            .await
            .map_err(gql_err)?;
        Ok(list.into_iter().map(|m| MilestoneGql::of(&p, m)).collect())
    }

    /// Status-Updates eines Projekts (neueste zuerst).
    async fn project_updates(
        &self,
        ctx: &Context<'_>,
        project_id: Option<String>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<ProjectUpdateGql>> {
        let (_, views) = app(ctx)?
            .list_project_updates(
                request_ctx(ctx)?,
                ListProjectUpdatesInput {
                    project: project_id,
                    include_archived: include_archived.unwrap_or(false),
                    page: PageRequest {
                        limit: i64::from(first.unwrap_or(50).clamp(1, 250)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(views.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct ProjectMutation;

#[Object]
impl ProjectMutation {
    async fn project_create(
        &self,
        ctx: &Context<'_>,
        input: ProjectInput,
    ) -> async_graphql::Result<ProjectPayload> {
        let v = app(ctx)?
            .save_project(request_ctx(ctx)?, project_input(None, input)?)
            .await
            .map_err(gql_err)?;
        Ok(ProjectPayload {
            success: true,
            project: Some(v.into()),
        })
    }

    async fn project_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: ProjectInput,
    ) -> async_graphql::Result<ProjectPayload> {
        let v = app(ctx)?
            .save_project(request_ctx(ctx)?, project_input(Some(id), input)?)
            .await
            .map_err(gql_err)?;
        Ok(ProjectPayload {
            success: true,
            project: Some(v.into()),
        })
    }

    /// Archiviert bzw. stellt ein Projekt wieder her (nur Admins und Lead).
    async fn project_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
        archived: Option<bool>,
    ) -> async_graphql::Result<ProjectPayload> {
        let v = app(ctx)?
            .set_project_archived(request_ctx(ctx)?, &id, archived.unwrap_or(true))
            .await
            .map_err(gql_err)?;
        Ok(ProjectPayload {
            success: true,
            project: Some(v.into()),
        })
    }

    /// Papierkorb (30 Tage) bzw. Wiederherstellen (nur per UUID).
    async fn project_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
        restore: Option<bool>,
    ) -> async_graphql::Result<ProjectSuccessPayload> {
        app(ctx)?
            .set_project_trashed(request_ctx(ctx)?, &id, !restore.unwrap_or(false))
            .await
            .map_err(gql_err)?;
        Ok(ProjectSuccessPayload { success: true })
    }

    async fn project_milestone_create(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        input: MilestoneInput,
    ) -> async_graphql::Result<ProjectMilestonePayload> {
        self.save_milestone(ctx, project_id, None, input).await
    }

    async fn project_milestone_update(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        id: String,
        input: MilestoneInput,
    ) -> async_graphql::Result<ProjectMilestonePayload> {
        self.save_milestone(ctx, project_id, Some(id), input).await
    }

    /// Löscht den Meilenstein; seine Issues bleiben im Projekt.
    async fn project_milestone_delete(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        id: String,
    ) -> async_graphql::Result<ProjectSuccessPayload> {
        app(ctx)?
            .delete_milestone(request_ctx(ctx)?, &project_id, &id)
            .await
            .map_err(gql_err)?;
        Ok(ProjectSuccessPayload { success: true })
    }

    /// Veröffentlicht ein Status-Update (`health`: onTrack | atRisk | offTrack).
    async fn project_update_create(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        body: String,
        health: Option<String>,
        is_diff_hidden: Option<bool>,
    ) -> async_graphql::Result<ProjectUpdatePayload> {
        let v = app(ctx)?
            .post_project_update(
                request_ctx(ctx)?,
                &project_id,
                &body,
                parse_health(health)?,
                is_diff_hidden.unwrap_or(false),
            )
            .await
            .map_err(gql_err)?;
        Ok(ProjectUpdatePayload {
            success: true,
            project_update: Some(v.into()),
        })
    }

    async fn project_update_edit(
        &self,
        ctx: &Context<'_>,
        id: String,
        body: Option<String>,
        health: Option<String>,
    ) -> async_graphql::Result<ProjectUpdatePayload> {
        let v = app(ctx)?
            .edit_project_update(
                request_ctx(ctx)?,
                &id,
                body.as_deref(),
                parse_health(health)?,
            )
            .await
            .map_err(gql_err)?;
        Ok(ProjectUpdatePayload {
            success: true,
            project_update: Some(v.into()),
        })
    }

    /// Löscht (archiviert) ein Status-Update.
    async fn project_update_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<ProjectSuccessPayload> {
        app(ctx)?
            .delete_project_update(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(ProjectSuccessPayload { success: true })
    }
}

impl ProjectMutation {
    async fn save_milestone(
        &self,
        ctx: &Context<'_>,
        project_id: String,
        id: Option<String>,
        input: MilestoneInput,
    ) -> async_graphql::Result<ProjectMilestonePayload> {
        let (p, m) = app(ctx)?
            .save_milestone(
                request_ctx(ctx)?,
                SaveMilestoneInput {
                    project: project_id,
                    id,
                    name: input.name,
                    description: input.description,
                    target_date: date("targetDate", input.target_date)?,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(ProjectMilestonePayload {
            success: true,
            project_milestone: Some(MilestoneGql::of(&p, m)),
        })
    }
}

pub(crate) fn parse_health(h: Option<String>) -> async_graphql::Result<Option<Health>> {
    h.map(|h| {
        h.parse::<Health>().map_err(|_| {
            gql_err(AppError::invalid_request(
                "`health` must be onTrack, atRisk or offTrack",
            ))
        })
    })
    .transpose()
}
