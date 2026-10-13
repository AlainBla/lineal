//! GraphQL-Teilobjekte des Bereichs `team` (WP02): Teams, Mitgliedschaften, Workflow-Status.
//!

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::types::Json;
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::state::{CreateStateInput, StateView, UpdateStateInput};
use lineal_app::usecases::team::{
    CreateTeamInput, ListTeamsInput, NestOptions, TeamView, UpdateTeamInput,
};
use lineal_domain::enums::{EstimationType, StateType, TeamVisibility};
use lineal_domain::ids::StateId;
use std::str::FromStr;

/// `Team`.
#[derive(SimpleObject)]
#[graphql(name = "Team", complex)]
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
    pub all_members_can_join: bool,
    pub timezone: String,
    pub triage_enabled: bool,
    pub cycles_enabled: bool,
    /// Cycle-Dauer in Wochen (1-8).
    pub cycle_duration: i32,
    /// Cooldown zwischen Cycles in Wochen (0-8).
    pub cycle_cooldown_time: i32,
    /// Starttag der Cycles (0 = Sonntag ... 6 = Samstag).
    pub cycle_start_day: i32,
    /// Anzahl künftig angelegter Cycles.
    pub upcoming_cycle_count: i32,
    pub cycle_issue_auto_assign_started: bool,
    pub cycle_issue_auto_assign_completed: bool,
    pub cycle_lock_to_active: bool,
    pub issue_estimation_type: String,
    pub inherit_workflow_statuses: bool,
    pub issue_estimation_extended: bool,
    pub issue_estimation_allow_zero: bool,
    pub default_issue_estimate: i32,
    pub inherit_issue_estimation: bool,
    /// Monate bis zum automatischen Schließen; `null` = aus.
    pub auto_close_period: Option<i32>,
    pub auto_close_state_id: Option<String>,
    /// Monate bis zum automatischen Archivieren.
    pub auto_archive_period: i32,
    pub auto_close_parent_issues: bool,
    pub auto_close_child_issues: bool,
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
            all_members_can_join: t.all_members_can_join,
            timezone: t.timezone,
            triage_enabled: t.triage_enabled,
            cycles_enabled: t.cycles_enabled,
            cycle_duration: t.cycle_duration,
            cycle_cooldown_time: t.cycle_cooldown_time,
            cycle_start_day: t.cycle_start_day,
            upcoming_cycle_count: t.upcoming_cycle_count,
            cycle_issue_auto_assign_started: t.cycle_issue_auto_assign_started,
            cycle_issue_auto_assign_completed: t.cycle_issue_auto_assign_completed,
            cycle_lock_to_active: t.cycle_lock_to_active,
            issue_estimation_type: t.issue_estimation_type.as_str().into(),
            inherit_workflow_statuses: t.inherit_workflow_statuses,
            issue_estimation_extended: t.issue_estimation_extended,
            issue_estimation_allow_zero: t.issue_estimation_allow_zero,
            default_issue_estimate: t.default_issue_estimate,
            inherit_issue_estimation: t.inherit_issue_estimation,
            auto_close_period: t.auto_close_period,
            auto_close_state_id: t.auto_close_state_id.map(|s| s.to_string()),
            auto_archive_period: t.auto_archive_period,
            auto_close_parent_issues: t.auto_close_parent_issues,
            auto_close_child_issues: t.auto_close_child_issues,
            retired_at: t.retired_at.map(|d| d.to_rfc3339()),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

#[async_graphql::ComplexObject]
impl TeamGql {
    /// Anzahl der Mitglieder; `null`, wenn der Aufrufer die Mitglieder nicht sehen darf.
    async fn member_count(&self, ctx: &Context<'_>) -> Option<i32> {
        let ctx_req = request_ctx(ctx).ok()?;
        let members = app(ctx)
            .ok()?
            .list_team_members(ctx_req, &self.id)
            .await
            .ok()?;
        i32::try_from(members.len()).ok()
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
    pub all_members_can_join: Option<bool>,
    pub timezone: Option<String>,
    pub triage_enabled: Option<bool>,
    pub copy_settings_from_team_id: Option<String>,
}

#[derive(InputObject)]
pub struct TeamUpdateInput {
    pub name: Option<String>,
    pub key: Option<String>,
    pub description: Option<String>,
    /// `null` entfernt das Icon, ein fehlendes Feld lässt es unverändert.
    pub icon: MaybeUndefined<String>,
    /// `null` entfernt die Farbe, ein fehlendes Feld lässt sie unverändert.
    pub color: MaybeUndefined<String>,
    pub timezone: Option<String>,
    pub visibility: Option<String>,
    pub all_members_can_join: Option<bool>,
    pub triage_enabled: Option<bool>,
    pub cycles_enabled: Option<bool>,
    /// `notUsed` | `exponential` | `fibonacci` | `linear` | `tShirt`.
    pub issue_estimation_type: Option<String>,
    pub issue_estimation_extended: Option<bool>,
    pub issue_estimation_allow_zero: Option<bool>,
    pub default_issue_estimate: Option<i32>,
    /// Monate (1, 3, 6, 9, 12); `0` schaltet das automatische Schließen aus.
    pub auto_close_period: Option<i32>,
    /// Zielstatus (Id oder Name) der Kategorie `canceled`; leerer Text setzt auf den Default zurück.
    pub auto_close_state_id: Option<String>,
    pub auto_archive_period: Option<i32>,
    pub auto_close_parent_issues: Option<bool>,
    pub auto_close_child_issues: Option<bool>,
}

#[derive(InputObject)]
pub struct NestTeamOptions {
    pub add_missing_members_to_parent: Option<bool>,
    pub downgrade_visibility: Option<bool>,
    pub inherit_workflow_statuses: Option<bool>,
    pub inherit_issue_estimation: Option<bool>,
    /// Eigene Zuordnung einzelner Status (eigener Status -> Status des Parents); der Rest folgt
    /// der automatischen Zuordnung.
    pub state_mapping: Option<Vec<StateMappingInput>>,
}

#[derive(InputObject)]
pub struct StateMappingInput {
    pub from_state_id: String,
    pub to_state_id: String,
}

/// Ein Konflikt der Nesting-Vorschau; `details` hängt von `kind` ab.
#[derive(SimpleObject)]
pub struct NestingConflictGql {
    /// `membership`, `visibility`, `workflowState`, `label`, `cycle`, `estimate`.
    pub kind: String,
    pub message: String,
    pub details: Json<serde_json::Value>,
}

#[derive(SimpleObject)]
pub struct NestingPreviewGql {
    pub conflicts: Vec<NestingConflictGql>,
}

/// Ein gelöschtes, noch wiederherstellbares Team.
#[derive(SimpleObject)]
pub struct DeletedTeamGql {
    pub id: String,
    pub key: String,
    pub name: String,
    pub display_name: String,
    pub parent_id: Option<String>,
    pub trashed_at: String,
    pub restorable_until: String,
    pub sub_team_count: i32,
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

/// Fehlend = unverändert, `null` = leeren, Wert = setzen.
fn patch_value<T>(v: MaybeUndefined<T>) -> Option<Option<T>> {
    match v {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(v) => Some(Some(v)),
    }
}

fn state_id(s: &str) -> async_graphql::Result<StateId> {
    uuid::Uuid::parse_str(s.trim())
        .map(StateId)
        .map_err(|_| gql_err(AppError::invalid_request("stateMapping needs status ids")))
}

fn nest_options(o: NestTeamOptions) -> async_graphql::Result<NestOptions> {
    let d = NestOptions::default();
    let mut mapping = Vec::new();
    for m in o.state_mapping.unwrap_or_default() {
        mapping.push((state_id(&m.from_state_id)?, state_id(&m.to_state_id)?));
    }
    Ok(NestOptions {
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
        state_mapping: mapping,
    })
}

fn visibility(s: &str) -> async_graphql::Result<TeamVisibility> {
    TeamVisibility::from_str(s).map_err(|e| gql_err(e.into()))
}

#[derive(Default)]
pub struct TeamQuery;

#[Object]
impl TeamQuery {
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

    /// Gelöschte Teams innerhalb der Papierkorb-Frist („Recently deleted“); nur für Workspace-Admins,
    /// sonst leer.
    async fn deleted_teams(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<DeletedTeamGql>> {
        let teams = app(ctx)?
            .list_deleted_teams(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(teams
            .into_iter()
            .map(|d| DeletedTeamGql {
                id: d.view.team.id.to_string(),
                key: d.view.team.key.clone(),
                name: d.view.team.name.clone(),
                display_name: d.view.display_name.clone(),
                parent_id: d.view.team.parent_id.map(|p| p.to_string()),
                trashed_at: d.trashed_at.to_rfc3339(),
                restorable_until: d.restorable_until.to_rfc3339(),
                sub_team_count: i32::try_from(d.sub_team_count).unwrap_or(i32::MAX),
            })
            .collect())
    }

    /// Vorschau auf `nestTeam`: Konflikte (Mitglieder, Sichtbarkeit, Status, Labels, Cycles,
    /// Schätzung) ohne zu schreiben. Nur Workspace-Admins.
    async fn preview_team_nesting(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        parent_id: Option<String>,
    ) -> async_graphql::Result<NestingPreviewGql> {
        let p = app(ctx)?
            .preview_team_nesting(request_ctx(ctx)?, &team_id, parent_id.as_deref())
            .await
            .map_err(gql_err)?;
        Ok(NestingPreviewGql {
            conflicts: p
                .conflicts
                .into_iter()
                .map(|c| NestingConflictGql {
                    kind: c.kind,
                    message: c.message,
                    details: Json(c.details),
                })
                .collect(),
        })
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
                    all_members_can_join: input.all_members_can_join,
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
        let estimation = input
            .issue_estimation_type
            .as_deref()
            .map(|s| {
                EstimationType::from_str(s).map_err(|_| {
                    async_graphql::Error::new(format!("invalid issueEstimationType `{s}`"))
                })
            })
            .transpose()?;
        let team = app(ctx)?
            .update_team(
                request_ctx(ctx)?,
                &id,
                UpdateTeamInput {
                    name: input.name,
                    key: input.key,
                    description: input.description.map(Some),
                    icon: patch_value(input.icon),
                    color: patch_value(input.color),
                    timezone: input.timezone,
                    visibility: vis,
                    all_members_can_join: input.all_members_can_join,
                    triage_enabled: input.triage_enabled,
                    cycles_enabled: input.cycles_enabled,
                    issue_estimation_type: estimation,
                    issue_estimation_extended: input.issue_estimation_extended,
                    issue_estimation_allow_zero: input.issue_estimation_allow_zero,
                    default_issue_estimate: input.default_issue_estimate,
                    auto_close_period: input.auto_close_period.map(|p| (p != 0).then_some(p)),
                    auto_close_state: input.auto_close_state_id,
                    auto_archive_period: input.auto_archive_period,
                    auto_close_parent_issues: input.auto_close_parent_issues,
                    auto_close_child_issues: input.auto_close_child_issues,
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
        let o = options.map(nest_options).transpose()?;
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
