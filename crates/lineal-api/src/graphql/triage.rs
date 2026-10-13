//! GraphQL-Teilobjekte des Bereichs `triage` (WP12, docs/03 §10).
//!
//! docs/09 kennt keine eigenen Triage-Mutationen (die Web-App nutzt `issueUpdate`); Lineal bietet
//! zusätzlich dedizierte Aktionen an, die Rechte, Zustandsprüfung (nur Issues in Triage) und
//! Kommentar in einer Transaktion bündeln: `issueTriageAccept`, `issueTriageDecline`,
//! `issueTriageDuplicate`, `issueTriageSnooze`, `issueTriageUnsnooze`, `issueTriageMove`.
//! Dazu `triageIssues`, `triageResponsibility` und die Pflege von Verantwortung und Schichtplänen.
//! Privates Team → `not_found`, Lese-Scope → `forbidden` (zentral in `App`).

use super::issue::{IssueGql, IssuePayload};
use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::ports::repos::triage::TriageAction;
use lineal_app::usecases::issue::IssueView;
use lineal_app::usecases::triage::{
    ScheduleEntryInput, SetTriageResponsibilityInput, ShiftView, TriageAcceptInput,
    TriageDeclineInput, TriageDuplicateInput, TriageInfo, TriageMoveInput, TriageSnoozeInput,
    UserBrief,
};

/// Kurzform eines Users (ohne E-Mail).
#[derive(SimpleObject)]
#[graphql(name = "TriageUser")]
pub struct TriageUserGql {
    pub id: String,
    pub name: String,
}

impl From<UserBrief> for TriageUserGql {
    fn from(u: UserBrief) -> Self {
        Self {
            id: u.id.to_string(),
            name: u.name,
        }
    }
}

/// Eine Schicht der Triage-Rotation.
#[derive(SimpleObject)]
#[graphql(name = "TriageShift")]
pub struct TriageShiftGql {
    pub user: TriageUserGql,
    pub starts_at: String,
    pub ends_at: String,
}

impl From<ShiftView> for TriageShiftGql {
    fn from(s: ShiftView) -> Self {
        Self {
            user: s.user.into(),
            starts_at: s.starts_at.to_rfc3339(),
            ends_at: s.ends_at.to_rfc3339(),
        }
    }
}

/// Wer für die Triage eines Teams verantwortlich ist.
#[derive(SimpleObject)]
#[graphql(name = "TriageResponsibility")]
pub struct TriageResponsibilityGql {
    pub team_id: String,
    pub triage_enabled: bool,
    /// `notify` oder `assign`.
    pub action: Option<String>,
    pub current_user: Option<TriageUserGql>,
    pub schedule_id: Option<String>,
    pub schedule_name: Option<String>,
    pub upcoming_shifts: Vec<TriageShiftGql>,
}

impl From<TriageInfo> for TriageResponsibilityGql {
    fn from(i: TriageInfo) -> Self {
        Self {
            team_id: i.team.id.to_string(),
            triage_enabled: i.triage_enabled,
            action: i.action.map(|a| a.as_str().to_owned()),
            current_user: i.current_user.map(Into::into),
            schedule_id: i.schedule.as_ref().map(|s| s.id.to_string()),
            schedule_name: i.schedule.map(|s| s.name),
            upcoming_shifts: i.upcoming_shifts.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(InputObject)]
pub struct TriageAcceptGqlInput {
    /// `backlog`-, `unstarted`- oder `started`-Status (Default: `defaultIssueState`).
    pub state_id: Option<String>,
    pub priority: Option<i32>,
    pub comment: Option<String>,
}

#[derive(InputObject)]
pub struct TriageDeclineGqlInput {
    /// `canceled`-Status (Default: erster `canceled`-Status).
    pub state_id: Option<String>,
    pub priority: Option<i32>,
    pub comment: Option<String>,
}

#[derive(InputObject)]
pub struct TriageResponsibilityGqlInput {
    pub team_id: String,
    /// `notify` (Default) oder `assign`.
    pub action: Option<String>,
    /// Manuelle Auswahl (Alternative zu `scheduleId`).
    pub user_ids: Option<Vec<String>>,
    pub schedule_id: Option<String>,
}

#[derive(InputObject)]
pub struct TimeScheduleEntryGqlInput {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub overridden_user_id: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "TimeSchedule")]
pub struct TimeScheduleGql {
    pub id: String,
    pub name: String,
    pub entry_count: i32,
}

fn payload(v: IssueView) -> IssuePayload {
    IssuePayload {
        success: true,
        issue: Some(v.into()),
    }
}

fn prio(p: Option<i32>) -> Option<i64> {
    p.map(i64::from)
}

fn action_of(s: Option<&str>) -> async_graphql::Result<Option<TriageAction>> {
    match s {
        None => Ok(None),
        Some(a) => TriageAction::parse(a).map(Some).ok_or_else(|| {
            gql_err(AppError::invalid_request(
                "action must be `notify` or `assign`",
            ))
        }),
    }
}

#[derive(Default)]
pub struct TriageQuery;

#[Object]
impl TriageQuery {
    /// Triage-Liste eines Teams (nicht gesnoozte, älteste zuerst).
    async fn triage_issues(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        include_snoozed: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<IssueGql>> {
        let items = app(ctx)?
            .list_triage_issues(
                request_ctx(ctx)?,
                &team_id,
                include_snoozed.unwrap_or(false),
                i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 50, 250),
                    ctx,
                )?),
            )
            .await
            .map_err(gql_err)?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    /// Triage-Verantwortung eines Teams: aktueller Verantwortlicher, Aktion, kommende Schichten.
    async fn triage_responsibility(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        user_id: Option<String>,
        first: Option<i32>,
    ) -> async_graphql::Result<TriageResponsibilityGql> {
        let info = app(ctx)?
            .get_triage_responsibility(
                request_ctx(ctx)?,
                &team_id,
                user_id.as_deref(),
                i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 10, 250),
                    ctx,
                )?),
            )
            .await
            .map_err(gql_err)?;
        Ok(info.into())
    }
}

#[derive(Default)]
pub struct TriageMutation;

#[Object]
impl TriageMutation {
    /// Accept: Status `defaultIssueState` oder gewählt, optionaler Kommentar.
    async fn issue_triage_accept(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: Option<TriageAcceptGqlInput>,
    ) -> async_graphql::Result<IssuePayload> {
        let input = input.unwrap_or(TriageAcceptGqlInput {
            state_id: None,
            priority: None,
            comment: None,
        });
        let v = app(ctx)?
            .triage_accept(
                request_ctx(ctx)?,
                TriageAcceptInput {
                    issue: id,
                    state: input.state_id,
                    priority: prio(input.priority),
                    comment: input.comment,
                    source: HistorySource::Api,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Decline: Status `canceled`, optionaler Begründungskommentar.
    async fn issue_triage_decline(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: Option<TriageDeclineGqlInput>,
    ) -> async_graphql::Result<IssuePayload> {
        let input = input.unwrap_or(TriageDeclineGqlInput {
            state_id: None,
            priority: None,
            comment: None,
        });
        let v = app(ctx)?
            .triage_decline(
                request_ctx(ctx)?,
                TriageDeclineInput {
                    issue: id,
                    state: input.state_id,
                    priority: prio(input.priority),
                    comment: input.comment,
                    source: HistorySource::Api,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Als Duplikat eines anderen Issues markieren (Anhänge wandern zum kanonischen Issue).
    async fn issue_triage_duplicate(
        &self,
        ctx: &Context<'_>,
        id: String,
        duplicate_of_id: String,
        comment: Option<String>,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .triage_duplicate(
                request_ctx(ctx)?,
                TriageDuplicateInput {
                    issue: id,
                    duplicate_of: duplicate_of_id,
                    comment,
                    source: HistorySource::Api,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Snooze bis `snoozedUntilAt` (höchstens ein Jahr).
    async fn issue_triage_snooze(
        &self,
        ctx: &Context<'_>,
        id: String,
        snoozed_until_at: DateTime<Utc>,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .triage_snooze(
                request_ctx(ctx)?,
                TriageSnoozeInput {
                    issue: id,
                    until: snoozed_until_at,
                    source: HistorySource::Api,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Beendet ein Snooze vorzeitig.
    async fn issue_triage_unsnooze(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .triage_unsnooze(request_ctx(ctx)?, &id, HistorySource::Api)
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// „Move to team“: im Zielteam gilt die Triage-Regel erneut.
    async fn issue_triage_move(
        &self,
        ctx: &Context<'_>,
        id: String,
        team_id: String,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .triage_move(
                request_ctx(ctx)?,
                TriageMoveInput {
                    issue: id,
                    team: team_id,
                    source: HistorySource::Api,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Setzt die Triage-Verantwortung (Personen oder Schichtplan).
    async fn triage_responsibility_set(
        &self,
        ctx: &Context<'_>,
        input: TriageResponsibilityGqlInput,
    ) -> async_graphql::Result<bool> {
        app(ctx)?
            .set_triage_responsibility(
                request_ctx(ctx)?,
                SetTriageResponsibilityInput {
                    team: input.team_id,
                    action: action_of(input.action.as_deref())?,
                    users: input.user_ids.unwrap_or_default(),
                    schedule: input.schedule_id,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(true)
    }

    /// Entfernt die Triage-Verantwortung eines Teams.
    async fn triage_responsibility_delete(
        &self,
        ctx: &Context<'_>,
        team_id: String,
    ) -> async_graphql::Result<bool> {
        app(ctx)?
            .delete_triage_responsibility(request_ctx(ctx)?, &team_id)
            .await
            .map_err(gql_err)
    }

    /// Legt einen Schichtplan an (Admin).
    async fn time_schedule_create(
        &self,
        ctx: &Context<'_>,
        name: String,
        entries: Vec<TimeScheduleEntryGqlInput>,
    ) -> async_graphql::Result<TimeScheduleGql> {
        let entries: Vec<ScheduleEntryInput> = entries
            .into_iter()
            .map(|e| ScheduleEntryInput {
                user: e.user_id,
                user_email: e.user_email,
                starts_at: e.starts_at,
                ends_at: e.ends_at,
                overridden_user: e.overridden_user_id,
            })
            .collect();
        let (schedule, stored) = app(ctx)?
            .create_time_schedule(request_ctx(ctx)?, &name, entries)
            .await
            .map_err(gql_err)?;
        Ok(TimeScheduleGql {
            id: schedule.id.to_string(),
            name: schedule.name,
            entry_count: i32::try_from(stored.len()).unwrap_or(i32::MAX),
        })
    }
}
