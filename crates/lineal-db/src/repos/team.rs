//! `TeamRepo` für `PgTx`: Teams, Key-Historie, Mitgliedschaften, Zähler.

use super::org::conflict_or_internal;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::TeamRepo;
use lineal_app::ports::repos::team::{Membership, NewMembership, NewTeam, Team, TeamPatch};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::{OrgId, StateId, TeamId, UserId};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct TeamRow {
    id: Uuid,
    organization_id: Uuid,
    name: String,
    key: String,
    description: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    parent_id: Option<Uuid>,
    visibility: String,
    all_members_can_join: bool,
    timezone: String,
    security_settings: Value,
    retired_at: Option<DateTime<Utc>>,
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    last_issue_number: i32,
    last_project_number: i32,
    last_cycle_number: i32,
    default_issue_state_id: Option<Uuid>,
    triage_enabled: bool,
    triage_issue_state_id: Option<Uuid>,
    require_priority_to_leave_triage: bool,
    auto_close_period: Option<i32>,
    auto_close_state_id: Option<Uuid>,
    auto_archive_period: i32,
    auto_close_parent_issues: bool,
    auto_close_child_issues: bool,
    set_issue_sort_order_on_state_change: String,
    group_issue_history: bool,
    inherit_workflow_statuses: bool,
    issue_estimation_type: String,
    issue_estimation_extended: bool,
    issue_estimation_allow_zero: bool,
    default_issue_estimate: i32,
    inherit_issue_estimation: bool,
    cycles_enabled: bool,
    cycle_duration: i32,
    cycle_cooldown_time: i32,
    cycle_start_day: i32,
    upcoming_cycle_count: i32,
    cycle_issue_auto_assign_started: bool,
    cycle_issue_auto_assign_completed: bool,
    cycle_lock_to_active: bool,
    inherit_project_statuses: bool,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const TEAM_COLS: &str = "id, organization_id, name, key, description, icon, color, parent_id, visibility, \
    all_members_can_join, timezone, security_settings, retired_at, trashed, trashed_at, last_issue_number, \
    last_project_number, last_cycle_number, default_issue_state_id, triage_enabled, triage_issue_state_id, \
    require_priority_to_leave_triage, auto_close_period, auto_close_state_id, auto_archive_period, \
    auto_close_parent_issues, auto_close_child_issues, set_issue_sort_order_on_state_change, \
    group_issue_history, inherit_workflow_statuses, issue_estimation_type, issue_estimation_extended, \
    issue_estimation_allow_zero, default_issue_estimate, inherit_issue_estimation, cycles_enabled, \
    cycle_duration, cycle_cooldown_time, cycle_start_day, upcoming_cycle_count, \
    cycle_issue_auto_assign_started, cycle_issue_auto_assign_completed, cycle_lock_to_active, \
    inherit_project_statuses, version, created_at, updated_at";

impl TeamRow {
    fn into_team(self) -> AppResult<Team> {
        let bad = |what: &str| AppError::internal(format!("invalid {what} in database"));
        Ok(Team {
            id: TeamId(self.id),
            org_id: OrgId(self.organization_id),
            name: self.name,
            key: self.key,
            description: self.description,
            icon: self.icon,
            color: self.color,
            parent_id: self.parent_id.map(TeamId),
            visibility: self.visibility.parse().map_err(|_| bad("visibility"))?,
            all_members_can_join: self.all_members_can_join,
            timezone: self.timezone,
            security_settings: self.security_settings,
            retired_at: self.retired_at,
            trashed: self.trashed,
            trashed_at: self.trashed_at,
            last_issue_number: self.last_issue_number,
            last_project_number: self.last_project_number,
            last_cycle_number: self.last_cycle_number,
            default_issue_state_id: self.default_issue_state_id.map(StateId),
            triage_enabled: self.triage_enabled,
            triage_issue_state_id: self.triage_issue_state_id.map(StateId),
            require_priority_to_leave_triage: self.require_priority_to_leave_triage,
            auto_close_period: self.auto_close_period,
            auto_close_state_id: self.auto_close_state_id.map(StateId),
            auto_archive_period: self.auto_archive_period,
            auto_close_parent_issues: self.auto_close_parent_issues,
            auto_close_child_issues: self.auto_close_child_issues,
            set_issue_sort_order_on_state_change: self.set_issue_sort_order_on_state_change,
            group_issue_history: self.group_issue_history,
            inherit_workflow_statuses: self.inherit_workflow_statuses,
            issue_estimation_type: self
                .issue_estimation_type
                .parse()
                .map_err(|_| bad("issue_estimation_type"))?,
            issue_estimation_extended: self.issue_estimation_extended,
            issue_estimation_allow_zero: self.issue_estimation_allow_zero,
            default_issue_estimate: self.default_issue_estimate,
            inherit_issue_estimation: self.inherit_issue_estimation,
            cycles_enabled: self.cycles_enabled,
            cycle_duration: self.cycle_duration,
            cycle_cooldown_time: self.cycle_cooldown_time,
            cycle_start_day: self.cycle_start_day,
            upcoming_cycle_count: self.upcoming_cycle_count,
            cycle_issue_auto_assign_started: self.cycle_issue_auto_assign_started,
            cycle_issue_auto_assign_completed: self.cycle_issue_auto_assign_completed,
            cycle_lock_to_active: self.cycle_lock_to_active,
            inherit_project_statuses: self.inherit_project_statuses,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct MembershipRow {
    id: Uuid,
    team_id: Uuid,
    user_id: Uuid,
    owner: bool,
    sort_order: f64,
    created_at: DateTime<Utc>,
}

impl From<MembershipRow> for Membership {
    fn from(r: MembershipRow) -> Self {
        Membership {
            id: r.id,
            team_id: TeamId(r.team_id),
            user_id: UserId(r.user_id),
            owner: r.owner,
            sort_order: r.sort_order,
            created_at: r.created_at,
        }
    }
}

const MEMBERSHIP_COLS: &str = "id, team_id, user_id, owner, sort_order, created_at";

/// Hängt `, col = $n` an, wenn der Patch den Wert enthält.
macro_rules! set_col {
    ($qb:expr, $col:literal, $val:expr) => {
        if let Some(v) = $val {
            $qb.push(concat!(", ", $col, " = ")).push_bind(v);
        }
    };
}

#[async_trait]
impl TeamRepo for PgTx {
    async fn insert(&mut self, t: &NewTeam) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO team (id, organization_id, name, key, description, icon, color, parent_id, visibility, \
             all_members_can_join, timezone, security_settings, triage_enabled, inherit_workflow_statuses, \
             inherit_issue_estimation, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $16)",
        )
        .bind(t.id.0)
        .bind(self.org.0)
        .bind(&t.name)
        .bind(&t.key)
        .bind(&t.description)
        .bind(&t.icon)
        .bind(&t.color)
        .bind(t.parent_id.map(|p| p.0))
        .bind(t.visibility.as_str())
        .bind(t.all_members_can_join)
        .bind(&t.timezone)
        .bind(&t.security_settings)
        .bind(t.triage_enabled)
        .bind(t.inherit_workflow_statuses)
        .bind(t.inherit_issue_estimation)
        .bind(t.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "team key or name already in use"))?;
        Ok(())
    }

    async fn get(&mut self, id: TeamId) -> AppResult<Option<Team>> {
        let row = sqlx::query_as::<_, TeamRow>(&format!(
            "SELECT {TEAM_COLS} FROM team WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(TeamRow::into_team).transpose()
    }

    async fn list_all(&mut self) -> AppResult<Vec<Team>> {
        let rows = sqlx::query_as::<_, TeamRow>(&format!(
            "SELECT {TEAM_COLS} FROM team WHERE organization_id = $1 ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(TeamRow::into_team).collect()
    }

    async fn update(&mut self, id: TeamId, p: &TeamPatch, now: DateTime<Utc>) -> AppResult<Team> {
        let mut qb: QueryBuilder<'_, Postgres> =
            QueryBuilder::new("UPDATE team SET version = version + 1, updated_at = ");
        qb.push_bind(now);
        set_col!(qb, "name", p.name.clone());
        set_col!(qb, "key", p.key.clone());
        set_col!(qb, "description", p.description.clone());
        set_col!(qb, "icon", p.icon.clone());
        set_col!(qb, "color", p.color.clone());
        set_col!(qb, "parent_id", p.parent_id.map(|o| o.map(|t| t.0)));
        set_col!(qb, "visibility", p.visibility.map(|v| v.as_str()));
        set_col!(qb, "all_members_can_join", p.all_members_can_join);
        set_col!(qb, "timezone", p.timezone.clone());
        set_col!(qb, "security_settings", p.security_settings.clone());
        set_col!(
            qb,
            "default_issue_state_id",
            p.default_issue_state_id.map(|o| o.map(|s| s.0))
        );
        set_col!(qb, "triage_enabled", p.triage_enabled);
        set_col!(
            qb,
            "triage_issue_state_id",
            p.triage_issue_state_id.map(|o| o.map(|s| s.0))
        );
        set_col!(
            qb,
            "require_priority_to_leave_triage",
            p.require_priority_to_leave_triage
        );
        set_col!(qb, "auto_close_period", p.auto_close_period);
        set_col!(
            qb,
            "auto_close_state_id",
            p.auto_close_state_id.map(|o| o.map(|s| s.0))
        );
        set_col!(qb, "auto_archive_period", p.auto_archive_period);
        set_col!(qb, "auto_close_parent_issues", p.auto_close_parent_issues);
        set_col!(qb, "auto_close_child_issues", p.auto_close_child_issues);
        set_col!(
            qb,
            "set_issue_sort_order_on_state_change",
            p.set_issue_sort_order_on_state_change.clone()
        );
        set_col!(qb, "group_issue_history", p.group_issue_history);
        set_col!(qb, "inherit_workflow_statuses", p.inherit_workflow_statuses);
        set_col!(
            qb,
            "issue_estimation_type",
            p.issue_estimation_type.map(|v| v.as_str())
        );
        set_col!(qb, "issue_estimation_extended", p.issue_estimation_extended);
        set_col!(
            qb,
            "issue_estimation_allow_zero",
            p.issue_estimation_allow_zero
        );
        set_col!(qb, "default_issue_estimate", p.default_issue_estimate);
        set_col!(qb, "inherit_issue_estimation", p.inherit_issue_estimation);
        set_col!(qb, "cycles_enabled", p.cycles_enabled);
        set_col!(qb, "cycle_duration", p.cycle_duration);
        set_col!(qb, "cycle_cooldown_time", p.cycle_cooldown_time);
        set_col!(qb, "cycle_start_day", p.cycle_start_day);
        set_col!(qb, "upcoming_cycle_count", p.upcoming_cycle_count);
        set_col!(
            qb,
            "cycle_issue_auto_assign_started",
            p.cycle_issue_auto_assign_started
        );
        set_col!(
            qb,
            "cycle_issue_auto_assign_completed",
            p.cycle_issue_auto_assign_completed
        );
        set_col!(qb, "cycle_lock_to_active", p.cycle_lock_to_active);
        set_col!(qb, "inherit_project_statuses", p.inherit_project_statuses);
        qb.push(" WHERE organization_id = ").push_bind(self.org.0);
        qb.push(" AND id = ").push_bind(id.0);
        qb.push(format!(" RETURNING {TEAM_COLS}"));
        let row = qb
            .build_query_as::<TeamRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|e| conflict_or_internal(e, "team key or name already in use"))?;
        row.ok_or_else(|| AppError::not_found("Could not find referenced Team."))?
            .into_team()
    }

    async fn key_reserved(&mut self, key: &str, except: Option<TeamId>) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM team WHERE organization_id = $1 AND key = $2 AND NOT trashed \
                            AND ($3::uuid IS NULL OR id <> $3)) \
                 OR EXISTS (SELECT 1 FROM team_key WHERE organization_id = $1 AND key = $2 \
                            AND ($3::uuid IS NULL OR team_id <> $3))",
        )
        .bind(self.org.0)
        .bind(key)
        .bind(except.map(|t| t.0))
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_id_by_key(&mut self, key: &str) -> AppResult<Option<TeamId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM ( \
               SELECT id, 0 AS prio FROM team WHERE organization_id = $1 AND key = $2 AND NOT trashed \
               UNION ALL \
               SELECT team_id, 1 FROM team_key WHERE organization_id = $1 AND key = $2 \
             ) c ORDER BY prio LIMIT 1",
        )
        .bind(self.org.0)
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id.map(TeamId))
    }

    async fn insert_key_history(
        &mut self,
        team: TeamId,
        key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO team_key (id, organization_id, team_id, key, replaced_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (organization_id, key) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(self.org.0)
        .bind(team.0)
        .bind(key)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "team key is reserved by another team"))?;
        Ok(())
    }

    async fn remove_key_history(&mut self, team: TeamId, key: &str) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM team_key WHERE organization_id = $1 AND team_id = $2 AND key = $3",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(key)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list_memberships(&mut self, team: TeamId) -> AppResult<Vec<Membership>> {
        let rows = sqlx::query_as::<_, MembershipRow>(&format!(
            "SELECT {MEMBERSHIP_COLS} FROM team_membership \
             WHERE organization_id = $1 AND team_id = $2 AND archived_at IS NULL ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Membership::from).collect())
    }

    async fn list_memberships_of_user(&mut self, user: UserId) -> AppResult<Vec<Membership>> {
        let rows = sqlx::query_as::<_, MembershipRow>(&format!(
            "SELECT {MEMBERSHIP_COLS} FROM team_membership \
             WHERE organization_id = $1 AND user_id = $2 AND archived_at IS NULL ORDER BY sort_order, created_at, id"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Membership::from).collect())
    }

    async fn get_membership(
        &mut self,
        team: TeamId,
        user: UserId,
    ) -> AppResult<Option<Membership>> {
        let row = sqlx::query_as::<_, MembershipRow>(&format!(
            "SELECT {MEMBERSHIP_COLS} FROM team_membership \
             WHERE organization_id = $1 AND team_id = $2 AND user_id = $3 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Membership::from))
    }

    async fn insert_membership(&mut self, m: &NewMembership) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO team_membership (id, organization_id, team_id, user_id, owner, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6)",
        )
        .bind(m.id)
        .bind(self.org.0)
        .bind(m.team_id.0)
        .bind(m.user_id.0)
        .bind(m.owner)
        .bind(m.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "user is already a member of this team"))?;
        Ok(())
    }

    async fn set_membership_owner(
        &mut self,
        team: TeamId,
        user: UserId,
        owner: bool,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let res = sqlx::query(
            "UPDATE team_membership SET owner = $4, version = version + 1, updated_at = $5 \
             WHERE organization_id = $1 AND team_id = $2 AND user_id = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(user.0)
        .bind(owner)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced membership."));
        }
        Ok(())
    }

    async fn delete_membership(&mut self, team: TeamId, user: UserId) -> AppResult<bool> {
        let res = sqlx::query(
            "DELETE FROM team_membership WHERE organization_id = $1 AND team_id = $2 AND user_id = $3",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(user.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn allocate_issue_numbers(&mut self, team: TeamId, count: i32) -> AppResult<i32> {
        if count < 1 {
            return Err(AppError::invalid_request("count must be at least 1"));
        }
        let last: Option<i32> = sqlx::query_scalar(
            "UPDATE team SET last_issue_number = last_issue_number + $3 \
             WHERE organization_id = $1 AND id = $2 AND NOT trashed AND retired_at IS NULL AND archived_at IS NULL \
             RETURNING last_issue_number",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(count)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        last.map(|l| l - count + 1)
            .ok_or_else(|| AppError::not_found("Could not find referenced Team."))
    }

    async fn next_project_number(&mut self, team: TeamId) -> AppResult<i32> {
        let n: Option<i32> = sqlx::query_scalar(
            "UPDATE team SET last_project_number = last_project_number + 1 \
             WHERE organization_id = $1 AND id = $2 AND NOT trashed AND retired_at IS NULL AND archived_at IS NULL \
             RETURNING last_project_number",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        n.ok_or_else(|| AppError::not_found("Could not find referenced Team."))
    }

    async fn next_cycle_number(&mut self, team: TeamId) -> AppResult<i32> {
        let n: Option<i32> = sqlx::query_scalar(
            "UPDATE team SET last_cycle_number = last_cycle_number + 1 \
             WHERE organization_id = $1 AND id = $2 AND NOT trashed AND retired_at IS NULL AND archived_at IS NULL \
             RETURNING last_cycle_number",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        n.ok_or_else(|| AppError::not_found("Could not find referenced Team."))
    }
}
