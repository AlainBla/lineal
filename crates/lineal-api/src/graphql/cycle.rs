//! GraphQL-Teilobjekte des Bereichs `cycle` (WP07): Cycles lesen, Einstellungen, Verschieben.

//! `progress` ist hier wie im Domänenmodell 0..1.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use chrono::DateTime;
use lineal_app::usecases::cycle::{
    CycleKind, CycleSettingsInput, CycleView, ExistingActiveIssues, UpdateCycleInput,
};

/// `Cycle` (Berechnete Flags: `isActive`, `isPast`, `isFuture`, `isNext`, `isPrevious`).
#[derive(SimpleObject)]
#[graphql(name = "Cycle")]
pub struct CycleGql {
    pub id: String,
    pub team_id: String,
    pub number: i32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub completed_at: Option<String>,
    pub inherited_from_id: Option<String>,
    pub progress: f64,
    pub scope: f64,
    pub completed_scope: f64,
    pub issue_count: i64,
    pub completed_issue_count: i64,
    pub scope_history: Vec<f64>,
    pub completed_scope_history: Vec<f64>,
    pub in_progress_scope_history: Vec<f64>,
    pub issue_count_history: Vec<f64>,
    pub completed_issue_count_history: Vec<f64>,
    pub is_active: bool,
    pub is_past: bool,
    pub is_future: bool,
    pub is_next: bool,
    pub is_previous: bool,
    /// Kapazitätsschätzung (Velocity der letzten drei Cycles); nur laufende und künftige Cycles.
    pub capacity: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<CycleView> for CycleGql {
    fn from(v: CycleView) -> Self {
        let c = v.cycle;
        Self {
            id: c.id.to_string(),
            team_id: c.team_id.to_string(),
            number: c.number,
            name: c.name,
            description: c.description,
            starts_at: c.starts_at.to_rfc3339(),
            ends_at: c.ends_at.to_rfc3339(),
            completed_at: c.completed_at.map(|d| d.to_rfc3339()),
            inherited_from_id: c.inherited_from_id.map(|i| i.to_string()),
            progress: v.progress,
            scope: v.scope.scope,
            completed_scope: v.scope.completed_scope,
            issue_count: v.scope.issue_count,
            completed_issue_count: v.scope.completed_issue_count,
            scope_history: c.scope_history,
            completed_scope_history: c.completed_scope_history,
            in_progress_scope_history: c.in_progress_scope_history,
            issue_count_history: c.issue_count_history,
            completed_issue_count_history: c.completed_issue_count_history,
            is_active: v.is_active,
            is_past: v.is_past,
            is_future: v.is_future,
            is_next: v.is_next,
            is_previous: v.is_previous,
            capacity: v.capacity,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        }
    }
}

/// Cycle-Einstellungen eines Teams (alle Felder optional).
#[derive(InputObject)]
pub struct TeamCyclesInput {
    pub cycles_enabled: Option<bool>,
    pub cycle_duration: Option<i32>,
    pub cycle_cooldown_time: Option<i32>,
    pub cycle_start_day: Option<i32>,
    pub upcoming_cycle_count: Option<i32>,
    pub cycle_issue_auto_assign_started: Option<bool>,
    pub cycle_issue_auto_assign_completed: Option<bool>,
    pub cycle_lock_to_active: Option<bool>,
    pub timezone: Option<String>,
    /// Startdatum des ersten Cycles (`YYYY-MM-DD`, Team-Zeitzone) beim Einschalten.
    pub first_cycle_start_date: Option<String>,
    /// `moveToBacklog` | `addToCycle` beim Einschalten von „Active issues“.
    pub existing_active_issues: Option<String>,
}

#[derive(InputObject)]
pub struct CycleUpdateInput {
    pub name: MaybeUndefined<String>,
    pub description: MaybeUndefined<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
}

#[derive(SimpleObject)]
pub struct CyclePayload {
    pub success: bool,
    pub cycle: Option<CycleGql>,
}

#[derive(SimpleObject)]
pub struct CyclesPayload {
    pub success: bool,
    pub cycles: Vec<CycleGql>,
}

#[derive(SimpleObject)]
pub struct CycleSuccessPayload {
    pub success: bool,
}

fn tri<T>(v: MaybeUndefined<T>) -> Option<Option<T>> {
    match v {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(x) => Some(Some(x)),
    }
}

fn time(field: &str, s: &str) -> async_graphql::Result<chrono::DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|_| {
            gql_err(
                lineal_app::AppError::invalid_request(format!(
                    "`{field}` must be an ISO-8601 timestamp"
                ))
                .with_details(serde_json::json!({ "field": field })),
            )
        })
}

#[derive(Default)]
pub struct CycleQuery;

#[Object]
impl CycleQuery {
    /// Cycles eines Teams (Key, Name oder UUID), aufsteigend nach Nummer; `type`:
    /// `current` | `previous` | `next`. Ohne aktivierte Cycles leer.
    async fn cycles(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        #[graphql(name = "type")] kind: Option<String>,
    ) -> async_graphql::Result<Vec<CycleGql>> {
        let kind = match kind.as_deref() {
            None => None,
            Some("current") => Some(CycleKind::Current),
            Some("previous") => Some(CycleKind::Previous),
            Some("next") => Some(CycleKind::Next),
            Some(_) => {
                return Err(gql_err(lineal_app::AppError::invalid_request(
                    "`type` must be current, previous or next",
                )));
            }
        };
        let (_, views) = app(ctx)?
            .list_cycles(request_ctx(ctx)?, &team_id, kind)
            .await
            .map_err(gql_err)?;
        Ok(views.into_iter().map(Into::into).collect())
    }

    /// Ein Cycle per UUID; Cycles privater Teams sind für Nicht-Mitglieder `not_found`.
    async fn cycle(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<CycleGql> {
        Ok(app(ctx)?
            .get_cycle(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?
            .into())
    }
}

#[derive(Default)]
pub struct CycleMutation;

#[Object]
impl CycleMutation {
    /// Cycle-Einstellungen eines Teams ändern (aktivieren, Zeitplan, „Active issues“, deaktivieren).
    async fn team_cycles_update(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        input: TeamCyclesInput,
    ) -> async_graphql::Result<CycleSuccessPayload> {
        let first = input
            .first_cycle_start_date
            .as_deref()
            .map(|s| {
                lineal_domain::rules::date::parse_iso_date("firstCycleStartDate", s).map_err(|_| {
                    gql_err(lineal_app::AppError::invalid_request(
                        "`firstCycleStartDate` must be YYYY-MM-DD",
                    ))
                })
            })
            .transpose()?;
        let existing = match input.existing_active_issues.as_deref() {
            None => None,
            Some("moveToBacklog") => Some(ExistingActiveIssues::MoveToBacklog),
            Some("addToCycle") => Some(ExistingActiveIssues::AddToCycle),
            Some(_) => {
                return Err(gql_err(lineal_app::AppError::invalid_request(
                    "`existingActiveIssues` must be moveToBacklog or addToCycle",
                )));
            }
        };
        app(ctx)?
            .configure_team_cycles(
                request_ctx(ctx)?,
                &team_id,
                CycleSettingsInput {
                    cycles_enabled: input.cycles_enabled,
                    duration: input.cycle_duration,
                    cooldown: input.cycle_cooldown_time,
                    start_day: input.cycle_start_day,
                    upcoming_count: input.upcoming_cycle_count,
                    auto_assign_started: input.cycle_issue_auto_assign_started,
                    auto_assign_completed: input.cycle_issue_auto_assign_completed,
                    lock_to_active: input.cycle_lock_to_active,
                    timezone: input.timezone,
                    first_start_date: first,
                    existing_active_issues: existing,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(CycleSuccessPayload { success: true })
    }

    /// Cycle umbenennen bzw. Beginn/Ende verschieben (nur künftige Cycles und das Ende des aktiven).
    async fn cycle_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: CycleUpdateInput,
    ) -> async_graphql::Result<CyclePayload> {
        let v = app(ctx)?
            .update_cycle(
                request_ctx(ctx)?,
                UpdateCycleInput {
                    id,
                    name: tri(input.name),
                    description: tri(input.description),
                    starts_at: input
                        .starts_at
                        .as_deref()
                        .map(|s| time("startsAt", s))
                        .transpose()?,
                    ends_at: input
                        .ends_at
                        .as_deref()
                        .map(|s| time("endsAt", s))
                        .transpose()?,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(CyclePayload {
            success: true,
            cycle: Some(v.into()),
        })
    }

    /// Verschiebt den angegebenen künftigen Cycle und alle späteren um `daysToShift` Tage.
    async fn cycle_shift_all(
        &self,
        ctx: &Context<'_>,
        id: String,
        days_to_shift: i32,
    ) -> async_graphql::Result<CyclesPayload> {
        let views = app(ctx)?
            .shift_cycles(request_ctx(ctx)?, &id, i64::from(days_to_shift))
            .await
            .map_err(gql_err)?;
        Ok(CyclesPayload {
            success: true,
            cycles: views.into_iter().map(Into::into).collect(),
        })
    }

    /// Startet den nächsten Cycle heute (laufender Cycle wird abgeschlossen, unfertige Issues rollen über).
    async fn cycle_start_upcoming_cycle_today(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CyclePayload> {
        let v = app(ctx)?
            .start_cycle_today(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(CyclePayload {
            success: true,
            cycle: Some(v.into()),
        })
    }

    /// Archiviert einen abgeschlossenen oder künftigen Cycle (Issues verlieren die Zuordnung).
    async fn cycle_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CycleSuccessPayload> {
        app(ctx)?
            .archive_cycle(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(CycleSuccessPayload { success: true })
    }
}
