//! GraphQL-Teilobjekte des Bereichs `team` (WP02): Teams, Mitgliedschaften, Workflow-Status.
//!
//! Das Platzhalterfeld `_teamReady` bleibt bestehen (die Scaffold-Tests prüfen es).

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::state::{CreateStateInput, StateView, UpdateStateInput};
use lineal_app::usecases::team::{
    CreateTeamInput, ListTeamsInput, NestOptions, TeamView, UpdateTeamInput,
};
use lineal_domain::enums::{StateType, TeamVisibility};
use lineal_domain::ids::StateId;
use std::str::FromStr;

/// `Team`.
#[derive(SimpleObject)]
#[graphql(name = "Team")]
pub struct TeamGql {
    pub id: String,
    pub key: String,
    pub name: String,
    pub display_name: String,
    pub parent_id: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub visibility: String,
    pub timezone: String,
    pub triage_enabled: bool,
    pub cycles_enabled: bool,
    pub issue_estimation_type: String,
    pub inherit_workflow_statuses: bool,
    pub retired_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<TeamView> for TeamGql {
    fn from(v: TeamView) -> Self {
        let t = v.team;
        Self {
            id: t.id.to_string(),
            key: t.key,
            name: t.name,
            display_name: v.display_name,
            parent_id: t.parent_id.map(|p| p.to_string()),
            description: t.description,
            icon: t.icon,
            color: t.color,
            visibility: t.visibility.as_str().into(),
            timezone: t.timezone,
            triage_enabled: t.triage_enabled,
            cycles_enabled: t.cycles_enabled,
            issue_estimation_type: t.issue_estimation_type.as_str().into(),
            inherit_workflow_statuses: t.inherit_workflow_statuses,
            retired_at: t.retired_at.map(|d| d.to_rfc3339()),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

/// `WorkflowState`.
#[derive(SimpleObject)]
#[graphql(name = "WorkflowState")]
pub struct WorkflowStateGql {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[graphql(name = "type")]
    pub state_type: String,
    pub color: String,
    pub description: Option<String>,
    pub position: f64,
    pub inherited_from_id: Option<String>,
    pub is_default: bool,
}

impl From<StateView> for WorkflowStateGql {
    fn from(v: StateView) -> Self {
        let s = v.state;
        Self {
            id: s.id.to_string(),
            team_id: s.team_id.to_string(),
            name: s.name,
            state_type: s.state_type.as_str().into(),
            color: s.color,
            description: s.description,
            position: s.position,
            inherited_from_id: s.inherited_from_id.map(|i| i.to_string()),
            is_default: v.is_default,
        }
    }
}

#[derive(InputObject)]
pub struct TeamCreateInput {
    pub name: String,
    pub key: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub parent_id: Option<String>,
    pub visibility: Option<String>,
    pub timezone: Option<String>,
    pub triage_enabled: Option<bool>,
    pub copy_settings_from_team_id: Option<String>,
}

#[derive(InputObject)]
pub struct TeamUpdateInput {
    pub name: Option<String>,
    pub key: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub timezone: Option<String>,
    pub visibility: Option<String>,
    pub triage_enabled: Option<bool>,
    pub cycles_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct NestTeamOptions {
    pub add_missing_members_to_parent: Option<bool>,
    pub downgrade_visibility: Option<bool>,
    pub inherit_workflow_statuses: Option<bool>,
    pub inherit_issue_estimation: Option<bool>,
}

#[derive(InputObject)]
pub struct WorkflowStateCreateInput {
    pub team_id: String,
    pub name: String,
    #[graphql(name = "type")]
    pub state_type: String,
    pub color: String,
    pub description: Option<String>,
    pub position: Option<f64>,
}

#[derive(InputObject)]
pub struct WorkflowStateUpdateInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
    pub position: Option<f64>,
}

#[derive(SimpleObject)]
pub struct TeamPayload {
    pub success: bool,
    pub team: Option<TeamGql>,
}

#[derive(SimpleObject)]
pub struct WorkflowStatePayload {
    pub success: bool,
    pub workflow_state: Option<WorkflowStateGql>,
}

#[derive(SimpleObject)]
pub struct SuccessPayload {
    pub success: bool,
}

fn visibility(s: &str) -> async_graphql::Result<TeamVisibility> {
    TeamVisibility::from_str(s).map_err(|e| gql_err(e.into()))
}

#[derive(Default)]
pub struct TeamQuery;

#[Object]
impl TeamQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_teamReady")]
    async fn team_ready(&self) -> bool {
        true
    }

    /// Sichtbare Teams (private Teams ohne Mitgliedschaft erscheinen nicht).
    async fn teams(
        &self,
        ctx: &Context<'_>,
        query: Option<String>,
        parent_team: Option<String>,
        ancestor_team: Option<String>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<TeamGql>> {
        let page = app(ctx)?
            .list_teams(
                request_ctx(ctx)?,
                ListTeamsInput {
                    query,
                    parent_team,
                    ancestor_team,
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
        Ok(page.items.into_iter().map(Into::into).collect())
    }

    /// Ein Team per UUID, Key oder Name; nicht sichtbar → `not_found`.
    async fn team(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<TeamGql> {
        Ok(app(ctx)?
            .get_team(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Workflow-Status eines Teams.
    async fn workflow_states(
        &self,
        ctx: &Context<'_>,
        team_id: String,
    ) -> async_graphql::Result<Vec<WorkflowStateGql>> {
        let states = app(ctx)?
            .list_workflow_states(request_ctx(ctx)?, &team_id)
            .await
            .map_err(gql_err)?;
        Ok(states.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct TeamMutation;

#[Object]
impl TeamMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_teamMutationReady")]
    async fn team_mutation_ready(&self) -> bool {
        true
    }

    async fn team_create(
        &self,
        ctx: &Context<'_>,
        input: TeamCreateInput,
    ) -> async_graphql::Result<TeamPayload> {
        let vis = input.visibility.as_deref().map(visibility).transpose()?;
        let team = app(ctx)?
            .create_team(
                request_ctx(ctx)?,
                CreateTeamInput {
                    name: input.name,
                    key: input.key,
                    description: input.description,
                    icon: input.icon,
                    color: input.color,
                    parent: input.parent_id,
                    visibility: vis,
                    timezone: input.timezone,
                    triage_enabled: input.triage_enabled,
                    copy_settings_from: input.copy_settings_from_team_id,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(TeamPayload {
            success: true,
            team: Some(team.into()),
        })
    }

    async fn team_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: TeamUpdateInput,
    ) -> async_graphql::Result<TeamPayload> {
        let vis = input.visibility.as_deref().map(visibility).transpose()?;
        let team = app(ctx)?
            .update_team(
                request_ctx(ctx)?,
                &id,
                UpdateTeamInput {
                    name: input.name,
                    key: input.key,
                    description: input.description.map(Some),
                    icon: input.icon.map(Some),
                    color: input.color.map(Some),
                    timezone: input.timezone,
                    visibility: vis,
                    triage_enabled: input.triage_enabled,
                    cycles_enabled: input.cycles_enabled,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(TeamPayload {
            success: true,
            team: Some(team.into()),
        })
    }

    /// Hängt ein Team unter einen Parent (`parentId = null`: Un-Nesting). Nur Workspace-Admins.
    async fn nest_team(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        parent_id: Option<String>,
        options: Option<NestTeamOptions>,
    ) -> async_graphql::Result<TeamPayload> {
        let d = NestOptions::default();
        let o = options.map(|o| NestOptions {
            add_missing_members_to_parent: o
                .add_missing_members_to_parent
                .unwrap_or(d.add_missing_members_to_parent),
            downgrade_visibility: o.downgrade_visibility.unwrap_or(d.downgrade_visibility),
            inherit_workflow_statuses: o
                .inherit_workflow_statuses
                .unwrap_or(d.inherit_workflow_statuses),
            inherit_issue_estimation: o
                .inherit_issue_estimation
                .unwrap_or(d.inherit_issue_estimation),
            state_mapping: Vec::new(),
        });
        let team = app(ctx)?
            .nest_team(
                request_ctx(ctx)?,
                &team_id,
                parent_id.as_deref(),
                o.unwrap_or_default(),
            )
            .await
            .map_err(gql_err)?;
        Ok(TeamPayload {
            success: true,
            team: Some(team.into()),
        })
    }

    async fn team_membership_create(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        user_id: String,
        owner: Option<bool>,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .add_team_member(
                request_ctx(ctx)?,
                &team_id,
                &user_id,
                owner.unwrap_or(false),
            )
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }

    async fn team_membership_delete(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        user_id: String,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .remove_team_member(request_ctx(ctx)?, &team_id, &user_id)
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }

    async fn workflow_state_create(
        &self,
        ctx: &Context<'_>,
        input: WorkflowStateCreateInput,
    ) -> async_graphql::Result<WorkflowStatePayload> {
        let state_type = StateType::from_str(&input.state_type).map_err(|e| gql_err(e.into()))?;
        let s = app(ctx)?
            .create_workflow_state(
                request_ctx(ctx)?,
                CreateStateInput {
                    team: input.team_id,
                    name: input.name,
                    state_type,
                    color: input.color,
                    description: input.description,
                    position: input.position,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(WorkflowStatePayload {
            success: true,
            workflow_state: Some(s.into()),
        })
    }

    async fn workflow_state_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: WorkflowStateUpdateInput,
    ) -> async_graphql::Result<WorkflowStatePayload> {
        let id = StateId::from_str(&id)
            .map_err(|_| gql_err(lineal_app::AppError::invalid_request("id must be a UUID")))?;
        let s = app(ctx)?
            .update_workflow_state(
                request_ctx(ctx)?,
                id,
                UpdateStateInput {
                    name: input.name,
                    color: input.color,
                    description: input.description.map(Some),
                    position: input.position,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(WorkflowStatePayload {
            success: true,
            workflow_state: Some(s.into()),
        })
    }

    /// Archiviert einen Status; mit Issues ist `targetStateId` Pflicht.
    async fn workflow_state_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
        target_state_id: Option<String>,
    ) -> async_graphql::Result<SuccessPayload> {
        let parse = |s: &str| {
            StateId::from_str(s)
                .map_err(|_| gql_err(lineal_app::AppError::invalid_request("id must be a UUID")))
        };
        let target = target_state_id.as_deref().map(parse).transpose()?;
        app(ctx)?
            .delete_workflow_state(request_ctx(ctx)?, parse(&id)?, target)
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }
}
