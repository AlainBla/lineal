//! GraphQL-Teilobjekte des Pakets WP30: SLA-Regeln und wiederkehrende Issues eines Teams.
//!
//! Rechte und Sichtbarkeit prüft `App` (private Teams → `not_found`, Lese-Scope → `forbidden`).

use super::label::IssueLabelGql;
use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::types::Json;
use async_graphql::{Context, InputObject, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::ports::repos::triage::{RecurringIssue, SlaRule};
use lineal_app::usecases::label::ListLabelsInput;
use lineal_app::usecases::team_extra::{
    CreateRecurringInput, CreateSlaRuleInput, SetTeamSlaInput, TeamSlaView, UpdateRecurringInput,
    UpdateSlaRuleInput,
};
use lineal_domain::enums::SlaType;
use serde_json::Value;
use uuid::Uuid;

fn uuid_of(s: &str, kind: &str) -> async_graphql::Result<Uuid> {
    Uuid::parse_str(s.trim()).map_err(|_| {
        gql_err(
            AppError::not_found(format!("Could not find referenced {kind}."))
                .with_details(serde_json::json!({ "type": kind })),
        )
    })
}

fn sla_type(s: &str) -> async_graphql::Result<SlaType> {
    s.parse::<SlaType>().map_err(|_| {
        gql_err(AppError::invalid_request(
            "slaType must be `all` or `onlyBusinessDays`",
        ))
    })
}

/// Eine SLA-Regel.
#[derive(SimpleObject)]
#[graphql(name = "SlaRule")]
pub struct SlaRuleGql {
    pub id: String,
    pub name: String,
    pub position: f64,
    /// `IssueFilter` (Priorität, Status, Assignee, Creator, Team, Projekt, Labels).
    pub filter: Json<Value>,
    /// `true` = die Regel entfernt ein SLA.
    pub removes_sla: bool,
    /// ISO-8601-Dauer (Millisekunden-Dauer bei `all`, Arbeitstage bei `onlyBusinessDays`).
    pub duration: Option<String>,
    pub sla_type: String,
    /// `issueCreation` oder `ruleMatch`.
    pub start_mode: String,
}

impl From<SlaRule> for SlaRuleGql {
    fn from(r: SlaRule) -> Self {
        Self {
            id: r.id.to_string(),
            name: r.name,
            position: r.position,
            filter: Json(r.filter),
            removes_sla: r.removes_sla,
            duration: r.duration,
            sla_type: r.sla_type.as_str().into(),
            start_mode: r.start_mode,
        }
    }
}

/// SLA-Einstellungen eines Teams.
#[derive(SimpleObject)]
#[graphql(name = "TeamSla")]
pub struct TeamSlaGql {
    pub team_id: String,
    pub enabled: bool,
    /// `monToFri` oder `sunToThu`.
    pub work_week: String,
    pub rules: Vec<SlaRuleGql>,
}

impl From<TeamSlaView> for TeamSlaGql {
    fn from(v: TeamSlaView) -> Self {
        Self {
            team_id: v.team_id.to_string(),
            enabled: v.enabled,
            work_week: if v.sun_thu { "sunToThu" } else { "monToFri" }.into(),
            rules: v.rules.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(InputObject)]
pub struct TeamSlaUpdateInput {
    pub enabled: Option<bool>,
    pub work_week: Option<String>,
}

#[derive(InputObject)]
pub struct SlaRuleCreateInput {
    pub team_id: String,
    pub name: String,
    pub filter: Json<Value>,
    #[graphql(default)]
    pub removes_sla: bool,
    pub duration: Option<String>,
    /// `all` (Default) oder `onlyBusinessDays`.
    pub sla_type: Option<String>,
    pub start_mode: Option<String>,
}

#[derive(InputObject)]
pub struct SlaRuleUpdateInput {
    pub name: Option<String>,
    pub filter: Option<Json<Value>>,
    pub removes_sla: Option<bool>,
    pub duration: Option<String>,
    pub sla_type: Option<String>,
    pub start_mode: Option<String>,
    pub position: Option<f64>,
}

/// Ein wiederkehrendes Issue.
#[derive(SimpleObject)]
#[graphql(name = "RecurringIssue")]
pub struct RecurringIssueGql {
    pub id: String,
    pub team_id: String,
    pub template_id: String,
    pub template_name: String,
    /// `daily`, `weekly` oder `monthly`.
    pub frequency: String,
    pub interval: i32,
    /// ISO-Wochentag 1 (Montag) bis 7 (Sonntag).
    pub weekday: Option<i32>,
    pub day_of_month: Option<i32>,
    /// Lokale Uhrzeit in Minuten seit Mitternacht (Team-Zeitzone).
    pub minute_of_day: i32,
    pub start_date: String,
    pub paused: bool,
    pub next_run_at: String,
    pub last_run_at: Option<String>,
    /// Fehler des letzten fehlgeschlagenen Laufs (z. B. Template gelöscht).
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub created_at: String,
}

impl From<RecurringIssue> for RecurringIssueGql {
    fn from(r: RecurringIssue) -> Self {
        Self {
            id: r.id.to_string(),
            team_id: r.team_id.to_string(),
            template_id: r.template_id.to_string(),
            template_name: r.template_name,
            frequency: r.frequency,
            interval: r.interval_count,
            weekday: r.weekday,
            day_of_month: r.day_of_month,
            minute_of_day: r.minute_of_day,
            start_date: r.start_date.to_string(),
            paused: r.paused,
            next_run_at: r.next_run_at.to_rfc3339(),
            last_run_at: r.last_run_at.map(|d| d.to_rfc3339()),
            last_error: r.last_error,
            last_error_at: r.last_error_at.map(|d| d.to_rfc3339()),
            created_at: r.created_at.to_rfc3339(),
        }
    }
}

#[derive(InputObject)]
pub struct RecurringIssueCreateInput {
    pub team_id: String,
    /// Issue-Template (UUID oder Name).
    pub template_id: String,
    pub frequency: String,
    pub interval: Option<i32>,
    pub weekday: Option<i32>,
    pub day_of_month: Option<i32>,
    pub minute_of_day: i32,
    /// `YYYY-MM-DD`; Default heute in der Team-Zeitzone.
    pub start_date: Option<String>,
}

#[derive(InputObject)]
pub struct RecurringIssueUpdateInput {
    pub template_id: Option<String>,
    pub frequency: Option<String>,
    pub interval: Option<i32>,
    pub weekday: Option<i32>,
    pub day_of_month: Option<i32>,
    pub minute_of_day: Option<i32>,
    pub paused: Option<bool>,
}

#[derive(SimpleObject)]
#[graphql(name = "TeamExtraSuccessPayload")]
pub struct SuccessPayload {
    pub success: bool,
}

#[derive(Default)]
pub struct TeamExtraQuery;

#[Object]
impl TeamExtraQuery {
    /// SLA-Einstellungen und Regeln eines Teams.
    async fn team_sla(
        &self,
        ctx: &Context<'_>,
        team_id: String,
    ) -> async_graphql::Result<TeamSlaGql> {
        Ok(app(ctx)?
            .get_team_sla(request_ctx(ctx)?, &team_id)
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Projekt-Labels des Workspace (für Projekt-Templates); gleiche Form wie `IssueLabel`.
    async fn project_labels(
        &self,
        ctx: &Context<'_>,
        include_groups: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<IssueLabelGql>> {
        let page = app(ctx)?
            .list_labels(
                request_ctx(ctx)?,
                LabelKind::Project,
                ListLabelsInput {
                    include_groups: include_groups.unwrap_or(true),
                    limit: i64::from(crate::error::GqlResultExt::gql(
                        crate::pagination::first_arg(first, 100, 250),
                        ctx,
                    )?),
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(page.items.into_iter().map(Into::into).collect())
    }

    /// Wiederkehrende Issues eines Teams.
    async fn recurring_issues(
        &self,
        ctx: &Context<'_>,
        team_id: String,
    ) -> async_graphql::Result<Vec<RecurringIssueGql>> {
        let list = app(ctx)?
            .list_recurring_issues(request_ctx(ctx)?, &team_id)
            .await
            .map_err(gql_err)?;
        Ok(list.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct TeamExtraMutation;

#[Object]
impl TeamExtraMutation {
    /// SLA ein-/ausschalten und Arbeitswoche setzen (legt beim ersten Aktivieren Standardregeln an).
    async fn team_sla_update(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        input: TeamSlaUpdateInput,
    ) -> async_graphql::Result<TeamSlaGql> {
        Ok(app(ctx)?
            .set_team_sla(
                request_ctx(ctx)?,
                SetTeamSlaInput {
                    team: team_id,
                    enabled: input.enabled,
                    work_week: input.work_week,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    async fn sla_rule_create(
        &self,
        ctx: &Context<'_>,
        input: SlaRuleCreateInput,
    ) -> async_graphql::Result<SlaRuleGql> {
        let sla = match input.sla_type.as_deref() {
            Some(s) => sla_type(s)?,
            None => SlaType::All,
        };
        Ok(app(ctx)?
            .create_sla_rule(
                request_ctx(ctx)?,
                CreateSlaRuleInput {
                    team: input.team_id,
                    name: input.name,
                    filter: input.filter.0,
                    removes_sla: input.removes_sla,
                    duration: input.duration,
                    sla_type: sla,
                    start_mode: input.start_mode,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    async fn sla_rule_update(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        id: String,
        input: SlaRuleUpdateInput,
    ) -> async_graphql::Result<SlaRuleGql> {
        let sla = input.sla_type.as_deref().map(sla_type).transpose()?;
        Ok(app(ctx)?
            .update_sla_rule(
                request_ctx(ctx)?,
                UpdateSlaRuleInput {
                    team: team_id,
                    id: uuid_of(&id, "SlaRule")?,
                    name: input.name,
                    filter: input.filter.map(|j| j.0),
                    removes_sla: input.removes_sla,
                    duration: input.duration,
                    sla_type: sla,
                    start_mode: input.start_mode,
                    position: input.position,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    async fn sla_rule_delete(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        id: String,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .delete_sla_rule(request_ctx(ctx)?, &team_id, uuid_of(&id, "SlaRule")?)
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }

    async fn recurring_issue_create(
        &self,
        ctx: &Context<'_>,
        input: RecurringIssueCreateInput,
    ) -> async_graphql::Result<RecurringIssueGql> {
        let start_date = input
            .start_date
            .as_deref()
            .map(|d| {
                lineal_domain::rules::date::parse_iso_date("startDate", d)
                    .map_err(|_| gql_err(AppError::invalid_request("startDate must be YYYY-MM-DD")))
            })
            .transpose()?;
        Ok(app(ctx)?
            .create_recurring_issue(
                request_ctx(ctx)?,
                CreateRecurringInput {
                    team: input.team_id,
                    template: input.template_id,
                    frequency: input.frequency,
                    interval: input.interval,
                    weekday: input.weekday,
                    day_of_month: input.day_of_month,
                    minute_of_day: input.minute_of_day,
                    start_date,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Rhythmus, Template oder Pause ändern.
    async fn recurring_issue_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: RecurringIssueUpdateInput,
    ) -> async_graphql::Result<RecurringIssueGql> {
        Ok(app(ctx)?
            .update_recurring_issue(
                request_ctx(ctx)?,
                UpdateRecurringInput {
                    id: uuid_of(&id, "RecurringIssue")?,
                    template: input.template_id,
                    frequency: input.frequency,
                    interval: input.interval,
                    weekday: input.weekday,
                    day_of_month: input.day_of_month,
                    minute_of_day: input.minute_of_day,
                    paused: input.paused,
                },
            )
            .await
            .map_err(gql_err)?
            .into())
    }

    async fn recurring_issue_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .delete_recurring_issue(request_ctx(ctx)?, uuid_of(&id, "RecurringIssue")?)
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }
}
