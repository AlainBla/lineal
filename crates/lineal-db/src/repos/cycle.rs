//! `CycleRepo` für `PgTx` (WP07).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::CycleRepo;
use lineal_app::ports::repos::cycle::{
    Cycle, CycleTeamRef, HistoryPoint, IssueScopeRow, NewCycle,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::{EstimationType, StateType};
use lineal_domain::ids::{CycleId, IssueId, OrgId, StateId, TeamId};
use serde_json::json;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct CycleRow {
    id: Uuid,
    organization_id: Uuid,
    team_id: Uuid,
    number: i32,
    name: Option<String>,
    description: Option<String>,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    inherited_from_id: Option<Uuid>,
    started_processed_at: Option<DateTime<Utc>>,
    progress: Option<f64>,
    uncompleted_issue_ids_upon_close: Option<Vec<Uuid>>,
    scope_history: Vec<f64>,
    completed_scope_history: Vec<f64>,
    in_progress_scope_history: Vec<f64>,
    issue_count_history: Vec<f64>,
    completed_issue_count_history: Vec<f64>,
    auto_archived_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const CYCLE_COLS: &str = "id, organization_id, team_id, number, name, description, starts_at, ends_at, \
    completed_at, inherited_from_id, started_processed_at, progress, uncompleted_issue_ids_upon_close, \
    scope_history, completed_scope_history, in_progress_scope_history, issue_count_history, \
    completed_issue_count_history, auto_archived_at, archived_at, version, created_at, updated_at";

impl From<CycleRow> for Cycle {
    fn from(r: CycleRow) -> Self {
        Cycle {
            id: CycleId(r.id),
            org_id: OrgId(r.organization_id),
            team_id: TeamId(r.team_id),
            number: r.number,
            name: r.name,
            description: r.description,
            starts_at: r.starts_at,
            ends_at: r.ends_at,
            completed_at: r.completed_at,
            inherited_from_id: r.inherited_from_id.map(CycleId),
            started_processed_at: r.started_processed_at,
            progress: r.progress,
            uncompleted_issue_ids: r.uncompleted_issue_ids_upon_close,
            scope_history: r.scope_history,
            completed_scope_history: r.completed_scope_history,
            in_progress_scope_history: r.in_progress_scope_history,
            issue_count_history: r.issue_count_history,
            completed_issue_count_history: r.completed_issue_count_history,
            auto_archived_at: r.auto_archived_at,
            archived_at: r.archived_at,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Zeile für Scope/Fortschritt (Issue ⨝ Team des Issues).
#[derive(sqlx::FromRow)]
pub(crate) struct ScopeRow {
    pub id: Uuid,
    pub team_id: Uuid,
    pub state_type: String,
    pub estimate: Option<i32>,
    pub issue_estimation_type: String,
    pub default_issue_estimate: i32,
}

impl ScopeRow {
    pub(crate) fn into_row(self) -> AppResult<IssueScopeRow> {
        Ok(IssueScopeRow {
            issue_id: IssueId(self.id),
            team_id: TeamId(self.team_id),
            state_type: self
                .state_type
                .parse::<StateType>()
                .map_err(|_| AppError::internal("invalid state type in database"))?,
            estimate: self.estimate,
            estimation_type: self
                .issue_estimation_type
                .parse::<EstimationType>()
                .map_err(|_| AppError::internal("invalid estimation type in database"))?,
            default_estimate: self.default_issue_estimate,
        })
    }
}

/// Bildet SQLSTATE 23P01 (Exclusion) und 23505 (Unique) auf `conflict` ab.
fn overlap_or_internal(e: sqlx::Error) -> AppError {
    let code = e
        .as_database_error()
        .and_then(|d| d.code().map(|c| c.to_string()));
    match code.as_deref() {
        Some("23P01") | Some("23505") => {
            AppError::conflict("The cycle would overlap another cycle of the team.")
        }
        _ => db_err(e),
    }
}

fn pad_set(arr: &mut Vec<f64>, idx: usize, value: f64) {
    let last = arr.last().copied().unwrap_or(0.0);
    while arr.len() < idx {
        arr.push(last);
    }
    if arr.len() == idx {
        arr.push(value);
    } else {
        arr[idx] = value;
    }
}

#[async_trait]
impl CycleRepo for PgTx {
    async fn insert(&mut self, c: &NewCycle) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO cycle (id, organization_id, team_id, number, name, description, starts_at, ends_at, \
             inherited_from_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
        )
        .bind(c.id.0)
        .bind(self.org.0)
        .bind(c.team_id.0)
        .bind(c.number)
        .bind(&c.name)
        .bind(&c.description)
        .bind(c.starts_at)
        .bind(c.ends_at)
        .bind(c.inherited_from_id.map(|i| i.0))
        .bind(c.now)
        .execute(&mut *self.tx)
        .await
        .map_err(overlap_or_internal)?;
        Ok(())
    }

    async fn get(&mut self, id: CycleId) -> AppResult<Option<Cycle>> {
        let row = sqlx::query_as::<_, CycleRow>(&format!(
            "SELECT {CYCLE_COLS} FROM cycle WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Cycle::from))
    }

    async fn list_for_team(&mut self, team: TeamId, include_archived: bool) -> AppResult<Vec<Cycle>> {
        let rows = sqlx::query_as::<_, CycleRow>(&format!(
            "SELECT {CYCLE_COLS} FROM cycle WHERE organization_id = $1 AND team_id = $2 \
             AND ($3 OR archived_at IS NULL) ORDER BY starts_at, id"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .bind(include_archived)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Cycle::from).collect())
    }

    async fn list_inheriting(&mut self, parent: CycleId) -> AppResult<Vec<Cycle>> {
        let rows = sqlx::query_as::<_, CycleRow>(&format!(
            "SELECT {CYCLE_COLS} FROM cycle WHERE organization_id = $1 AND inherited_from_id = $2 \
             ORDER BY starts_at, id"
        ))
        .bind(self.org.0)
        .bind(parent.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Cycle::from).collect())
    }

    async fn update_meta(
        &mut self,
        id: CycleId,
        name: Option<Option<String>>,
        description: Option<Option<String>>,
        now: DateTime<Utc>,
    ) -> AppResult<Cycle> {
        let row = sqlx::query_as::<_, CycleRow>(&format!(
            "UPDATE cycle SET \
               name = CASE WHEN $3 THEN $4 ELSE name END, \
               description = CASE WHEN $5 THEN $6 ELSE description END, \
               version = version + 1, updated_at = $7 \
             WHERE organization_id = $1 AND id = $2 RETURNING {CYCLE_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(name.is_some())
        .bind(name.flatten())
        .bind(description.is_some())
        .bind(description.flatten())
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(Cycle::from)
            .ok_or_else(|| AppError::not_found("Could not find referenced Cycle."))
    }

    async fn update_times(
        &mut self,
        id: CycleId,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Cycle> {
        let row = sqlx::query_as::<_, CycleRow>(&format!(
            "UPDATE cycle SET starts_at = $3, ends_at = $4, version = version + 1, updated_at = $5 \
             WHERE organization_id = $1 AND id = $2 RETURNING {CYCLE_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(starts_at)
        .bind(ends_at)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(overlap_or_internal)?;
        row.map(Cycle::from)
            .ok_or_else(|| AppError::not_found("Could not find referenced Cycle."))
    }

    async fn set_inherited_from(
        &mut self,
        id: CycleId,
        parent: Option<CycleId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE cycle SET inherited_from_id = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(parent.map(|p| p.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete(&mut self, id: CycleId) -> AppResult<()> {
        sqlx::query("DELETE FROM cycle WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn archive(&mut self, id: CycleId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE cycle SET archived_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn issue_rows(&mut self, cycle: CycleId) -> AppResult<Vec<IssueScopeRow>> {
        let rows = sqlx::query_as::<_, ScopeRow>(
            "SELECT i.id, i.team_id, i.state_type, i.estimate, t.issue_estimation_type, t.default_issue_estimate \
             FROM issue i JOIN team t ON t.organization_id = i.organization_id AND t.id = i.team_id \
             WHERE i.organization_id = $1 AND i.cycle_id = $2 AND NOT i.trashed",
        )
        .bind(self.org.0)
        .bind(cycle.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ScopeRow::into_row).collect()
    }

    async fn open_issue_ids(&mut self, cycle: CycleId) -> AppResult<Vec<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND cycle_id = $2 AND NOT trashed \
             AND state_type IN ('triage','backlog','unstarted','started') ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(cycle.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn issue_states_in_cycle(
        &mut self,
        cycle: CycleId,
        ids: &[Uuid],
    ) -> AppResult<Vec<(IssueId, StateType)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, state_type FROM issue WHERE organization_id = $1 AND cycle_id = $2 \
             AND id = ANY($3) ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(cycle.0)
        .bind(ids)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|(id, t)| {
                t.parse::<StateType>()
                    .map(|t| (IssueId(id), t))
                    .map_err(|_| AppError::internal("invalid state type in database"))
            })
            .collect()
    }

    async fn all_issue_ids(&mut self, cycle: CycleId) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND cycle_id = $2 ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(cycle.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn set_issue_cycle(
        &mut self,
        issue: IssueId,
        from: Option<CycleId>,
        to: Option<CycleId>,
        now: DateTime<Utc>,
    ) -> AppResult<TeamId> {
        let team: Option<Uuid> = sqlx::query_scalar(
            "UPDATE issue SET cycle_id = $3, added_to_cycle_at = CASE WHEN $3::uuid IS NULL THEN added_to_cycle_at ELSE $4 END, \
             version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2 RETURNING team_id",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(to.map(|c| c.0))
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let team = team.ok_or_else(|| AppError::not_found("Could not find referenced Issue."))?;
        sqlx::query(
            "INSERT INTO issue_history (organization_id, issue_id, actor_id, source, changes, created_at) \
             VALUES ($1, $2, NULL, 'system', $3, $4)",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(json!({ "cycleId": { "from": from.map(|c| c.0), "to": to.map(|c| c.0) } }))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(TeamId(team))
    }

    async fn write_history_point(
        &mut self,
        id: CycleId,
        idx: usize,
        point: HistoryPoint,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let Some(mut c) = CycleRepo::get(self, id).await? else {
            return Ok(());
        };
        if c.completed_at.is_some() {
            return Ok(());
        }
        pad_set(&mut c.scope_history, idx, point.scope);
        pad_set(&mut c.completed_scope_history, idx, point.completed_scope);
        pad_set(&mut c.in_progress_scope_history, idx, point.in_progress_scope);
        pad_set(&mut c.issue_count_history, idx, point.issue_count);
        pad_set(
            &mut c.completed_issue_count_history,
            idx,
            point.completed_issue_count,
        );
        sqlx::query(
            "UPDATE cycle SET scope_history = $3, completed_scope_history = $4, in_progress_scope_history = $5, \
             issue_count_history = $6, completed_issue_count_history = $7, updated_at = $8 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(&c.scope_history)
        .bind(&c.completed_scope_history)
        .bind(&c.in_progress_scope_history)
        .bind(&c.issue_count_history)
        .bind(&c.completed_issue_count_history)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn close(
        &mut self,
        id: CycleId,
        completed_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        uncompleted: &[Uuid],
        progress: f64,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE cycle SET completed_at = $3, ends_at = $4, uncompleted_issue_ids_upon_close = $5, \
             progress = $6, version = version + 1, updated_at = $7 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(completed_at)
        .bind(ends_at)
        .bind(uncompleted)
        .bind(progress)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(overlap_or_internal)?;
        Ok(())
    }

    async fn mark_started(&mut self, id: CycleId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE cycle SET started_processed_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn teams_with_cycles(&mut self, now: DateTime<Utc>) -> AppResult<Vec<CycleTeamRef>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT organization_id, id FROM team WHERE cycles_enabled AND NOT trashed AND retired_at IS NULL \
             AND archived_at IS NULL AND (import_locked_until IS NULL OR import_locked_until <= $1) \
             ORDER BY organization_id, id",
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(o, t)| CycleTeamRef {
                org_id: OrgId(o),
                team_id: TeamId(t),
            })
            .collect())
    }

    async fn lock_team(&mut self, team: TeamId) -> AppResult<()> {
        sqlx::query("SELECT 1 FROM team WHERE organization_id = $1 AND id = $2 FOR UPDATE")
            .bind(self.org.0)
            .bind(team.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn teams_not_inheriting(&mut self) -> AppResult<Vec<TeamId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM team WHERE organization_id = $1 AND NOT inherits_cycles",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(TeamId).collect())
    }

    async fn team_schedule_fp(&mut self, team: TeamId) -> AppResult<Option<String>> {
        sqlx::query_scalar("SELECT cycle_schedule_fp FROM team WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(team.0)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)
            .map(Option::flatten)
    }

    async fn set_team_schedule_fp(&mut self, team: TeamId, fp: Option<&str>) -> AppResult<()> {
        sqlx::query("UPDATE team SET cycle_schedule_fp = $3 WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(team.0)
            .bind(fp)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn teams_disabled_with_open_cycles(&mut self) -> AppResult<Vec<CycleTeamRef>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT t.organization_id, t.id FROM team t WHERE NOT t.cycles_enabled AND NOT t.trashed \
             AND EXISTS (SELECT 1 FROM cycle c WHERE c.organization_id = t.organization_id AND c.team_id = t.id \
                         AND c.completed_at IS NULL AND c.archived_at IS NULL) \
             ORDER BY t.organization_id, t.id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(o, t)| CycleTeamRef {
                org_id: OrgId(o),
                team_id: TeamId(t),
            })
            .collect())
    }

    async fn set_team_inherits(
        &mut self,
        team: TeamId,
        inherits: bool,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET inherits_cycles = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(inherits)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn assign_active_issues_without_cycle(
        &mut self,
        team: TeamId,
        cycle: CycleId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2 AND cycle_id IS NULL AND NOT trashed \
             AND archived_at IS NULL AND state_type IN ('unstarted','started') ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            CycleRepo::set_issue_cycle(self, IssueId(id), None, Some(cycle), now).await?;
            out.push(IssueId(id));
        }
        Ok(out)
    }

    async fn move_active_issues_without_cycle_to_backlog(
        &mut self,
        team: TeamId,
        backlog_state: StateId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT id, state_id FROM issue WHERE organization_id = $1 AND team_id = $2 AND cycle_id IS NULL \
             AND NOT trashed AND archived_at IS NULL AND state_type IN ('unstarted','started') ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, old_state) in rows {
            sqlx::query(
                "UPDATE issue SET state_id = $3, state_entered_at = $4, \
                 version = version + 1, updated_at = $4 WHERE organization_id = $1 AND id = $2",
            )
            .bind(self.org.0)
            .bind(id)
            .bind(backlog_state.0)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            sqlx::query(
                "INSERT INTO issue_history (organization_id, issue_id, actor_id, source, changes, created_at) \
                 VALUES ($1, $2, NULL, 'system', $3, $4)",
            )
            .bind(self.org.0)
            .bind(id)
            .bind(json!({ "stateId": { "from": old_state, "to": backlog_state.0 } }))
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            out.push(IssueId(id));
        }
        Ok(out)
    }
}
