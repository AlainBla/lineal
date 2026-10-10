//! GraphQL-Teilobjekte des Bereichs `overview` (WP11, docs/14 §4): `workspaceOverview`,
//! `workload`, `activeCycles`. Gäste erhalten `forbidden`; private Teams ohne Mitgliedschaft
//! tauchen weder in Zeilen noch in Summen auf. Die Zahlen kommen aus denselben Use-Cases wie
//! MCP `get_workspace_overview`.

use crate::context::{app, request_ctx};
use crate::error::GqlResultExt;
use async_graphql::{Context, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::usecases::overview::{
    ActiveCycleInfo, Counts, OverviewInput, OverviewPeriod, ProjectHealth, WorkloadEntry,
    WorkspaceOverview,
};

#[derive(SimpleObject)]
pub struct OverviewCounts {
    pub open_issues: i64,
    pub in_progress: i64,
    pub triage: i64,
    pub triage_oldest_at: Option<String>,
    pub completed: i64,
    pub completed_points: f64,
    pub overdue: i64,
    pub sla_at_risk: i64,
    pub sla_breached: i64,
}

impl From<&Counts> for OverviewCounts {
    fn from(c: &Counts) -> Self {
        Self {
            open_issues: c.open_issues,
            in_progress: c.in_progress,
            triage: c.triage,
            triage_oldest_at: c
                .triage_oldest_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            completed: c.completed,
            completed_points: c.completed_points,
            overdue: c.overdue,
            sla_at_risk: c.sla_at_risk,
            sla_breached: c.sla_breached,
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "ProjectHealthSummary")]
pub struct ProjectHealthGql {
    pub on_track: i64,
    pub at_risk: i64,
    pub off_track: i64,
    pub no_update: i64,
    pub total: i64,
}

impl From<&ProjectHealth> for ProjectHealthGql {
    fn from(h: &ProjectHealth) -> Self {
        Self {
            on_track: h.on_track,
            at_risk: h.at_risk,
            off_track: h.off_track,
            no_update: h.no_update,
            total: h.total(),
        }
    }
}

#[derive(SimpleObject)]
pub struct ThroughputWeek {
    pub week_offset: i32,
    pub issues: i64,
    pub points: f64,
}

#[derive(SimpleObject)]
pub struct TeamMetrics {
    pub team_id: String,
    pub key: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub counts: OverviewCounts,
    pub throughput: Vec<ThroughputWeek>,
    pub cycle_progress: Option<f64>,
    pub projects: ProjectHealthGql,
}

#[derive(SimpleObject)]
pub struct InitiativeSummary {
    pub id: String,
    pub identifier: String,
    pub name: String,
    pub status: String,
    pub health: Option<String>,
    pub project_count: i64,
    pub completed_project_count: i64,
}

#[derive(SimpleObject)]
pub struct IssueRef {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub team_key: String,
    pub due_date: Option<String>,
    pub sla_breaches_at: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(SimpleObject)]
pub struct WorkspaceOverviewGql {
    pub period: String,
    pub include_sub_teams: bool,
    pub teams: Vec<TeamMetrics>,
    pub totals: OverviewCounts,
    pub projects: ProjectHealthGql,
    pub initiatives: Vec<InitiativeSummary>,
    pub at_risk: Vec<IssueRef>,
}

impl From<WorkspaceOverview> for WorkspaceOverviewGql {
    fn from(o: WorkspaceOverview) -> Self {
        Self {
            period: o.period.as_str().into(),
            include_sub_teams: o.include_sub_teams,
            totals: (&o.totals).into(),
            projects: (&o.projects).into(),
            teams: o
                .teams
                .iter()
                .map(|t| TeamMetrics {
                    team_id: t.team_id.to_string(),
                    key: t.key.clone(),
                    name: t.name.clone(),
                    parent_id: t.parent_id.map(|p| p.to_string()),
                    depth: t.depth as i32,
                    counts: (&t.counts).into(),
                    throughput: t
                        .throughput
                        .iter()
                        .map(|w| ThroughputWeek {
                            week_offset: w.week_offset,
                            issues: w.issues,
                            points: w.points,
                        })
                        .collect(),
                    cycle_progress: t.cycle_progress,
                    projects: (&t.projects).into(),
                })
                .collect(),
            initiatives: o
                .initiatives
                .into_iter()
                .map(|i| InitiativeSummary {
                    id: i.id,
                    identifier: i.identifier,
                    name: i.name,
                    status: i.status,
                    health: i.health,
                    project_count: i.project_count,
                    completed_project_count: i.completed_project_count,
                })
                .collect(),
            at_risk: o
                .at_risk
                .into_iter()
                .map(|i| IssueRef {
                    id: i.id,
                    identifier: i.identifier,
                    title: i.title,
                    team_key: i.team_key,
                    due_date: i.due_date.map(|d| d.to_string()),
                    sla_breaches_at: i
                        .sla_breaches_at
                        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                    reasons: i.reasons,
                })
                .collect(),
        }
    }
}

#[derive(SimpleObject)]
pub struct WorkloadTeamEntry {
    pub team_id: String,
    pub team_key: String,
    pub team_name: String,
    pub open_issues: i64,
    pub points: f64,
}

#[derive(SimpleObject)]
pub struct WorkloadUser {
    pub id: String,
    pub name: String,
    pub display_name: String,
}

#[derive(SimpleObject)]
pub struct WorkloadRowGql {
    pub user: WorkloadUser,
    pub per_team: Vec<WorkloadTeamEntry>,
    pub total_open_issues: i64,
    pub total_points: f64,
}

impl From<WorkloadEntry> for WorkloadRowGql {
    fn from(e: WorkloadEntry) -> Self {
        Self {
            user: WorkloadUser {
                id: e.user_id.to_string(),
                name: e.name,
                display_name: e.display_name,
            },
            per_team: e
                .per_team
                .into_iter()
                .map(|t| WorkloadTeamEntry {
                    team_id: t.team_id.to_string(),
                    team_key: t.team_key,
                    team_name: t.team_name,
                    open_issues: t.open_issues,
                    points: t.points,
                })
                .collect(),
            total_open_issues: e.total_open_issues,
            total_points: e.total_points,
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "ActiveCycle")]
pub struct ActiveCycleGql {
    pub team_id: String,
    pub team_key: String,
    pub team_name: String,
    pub cycle_id: String,
    pub number: i32,
    pub name: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub remaining_days: i64,
    pub progress: f64,
    pub scope: f64,
    pub completed_scope: f64,
    pub in_progress_scope: f64,
    pub scope_change: i64,
}

impl From<ActiveCycleInfo> for ActiveCycleGql {
    fn from(c: ActiveCycleInfo) -> Self {
        Self {
            team_id: c.team_id.to_string(),
            team_key: c.team_key,
            team_name: c.team_name,
            cycle_id: c.cycle_id,
            number: c.number,
            name: c.name,
            starts_at: c.starts_at.to_rfc3339(),
            ends_at: c.ends_at.to_rfc3339(),
            remaining_days: c.remaining_days,
            progress: c.progress,
            scope: c.scope,
            completed_scope: c.completed_scope,
            in_progress_scope: c.in_progress_scope,
            scope_change: c.scope_change,
        }
    }
}

#[derive(Default)]
pub struct OverviewQuery;

#[Object]
impl OverviewQuery {
    /// Dashboard der Firma: Kennzahlen je Team (mit Rollup über Sub-Teams), Projekte nach
    /// Health, Initiativen, überfällige/SLA-gefährdete Issues. Gäste: `forbidden`.
    async fn workspace_overview(
        &self,
        ctx: &Context<'_>,
        team_ids: Option<Vec<String>>,
        include_sub_teams: Option<bool>,
        period: Option<String>,
    ) -> async_graphql::Result<WorkspaceOverviewGql> {
        let period = match period.as_deref() {
            None => OverviewPeriod::Days7,
            Some(p) => OverviewPeriod::parse(p)
                .ok_or_else(|| AppError::invalid_request("period must be 7d or 30d"))
                .gql(ctx)?,
        };
        Ok(app(ctx)?
            .workspace_overview(
                request_ctx(ctx)?,
                OverviewInput {
                    teams: team_ids.unwrap_or_default(),
                    include_sub_teams: include_sub_teams.unwrap_or(true),
                    period,
                },
            )
            .await
            .gql(ctx)?
            .into())
    }

    /// Auslastung: Person × Team (offene Issues und Punkte) im aktuellen (`current`) oder
    /// nächsten (`next`) Cycle.
    async fn workload(
        &self,
        ctx: &Context<'_>,
        team_ids: Option<Vec<String>>,
        cycle: Option<String>,
    ) -> async_graphql::Result<Vec<WorkloadRowGql>> {
        let next = match cycle.as_deref() {
            None | Some("current") => false,
            Some("next") => true,
            Some(_) => {
                return Err(AppError::invalid_request("cycle must be current or next")).gql(ctx);
            }
        };
        Ok(app(ctx)?
            .workload(request_ctx(ctx)?, team_ids.unwrap_or_default(), next)
            .await
            .gql(ctx)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Aktive Cycles aller (ausgewählten) sichtbaren Teams.
    async fn active_cycles(
        &self,
        ctx: &Context<'_>,
        team_ids: Option<Vec<String>>,
    ) -> async_graphql::Result<Vec<ActiveCycleGql>> {
        Ok(app(ctx)?
            .active_cycles(request_ctx(ctx)?, team_ids.unwrap_or_default())
            .await
            .gql(ctx)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}

#[derive(Default)]
pub struct OverviewMutation;

#[Object]
impl OverviewMutation {
    /// Platzhalter des Scaffolds (der Mutation-Root darf nicht leer sein).
    #[graphql(name = "_overviewMutationReady")]
    async fn overview_mutation_ready(&self) -> bool {
        true
    }
}
