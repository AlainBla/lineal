//! `TriageRepo` für `PgTx` (WP12): Triage-Verantwortung, Schichtpläne, Automationsspalten am
//! Issue, SLA-Regeln und Kandidatenabfragen der Jobs.
//!
//! Alle Abfragen filtern auf `organization_id`, außer den ausdrücklich als mandantenübergreifend
//! gekennzeichneten Listen (nur über `Store::begin_system`).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::ports::repos::TriageRepo;
use lineal_app::ports::repos::triage::{
    AutoCloseCandidate, IssueAutomation, NewRecurringIssue, NewResponsibility, NewScheduleEntry,
    NewSlaRule, RecurringIssue, ScheduleEntry, SlaMarker, SlaRule, TeamRef, TeamSla, TimeSchedule,
    TriageAction, TriageResponsibility,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::SlaType;
use lineal_domain::ids::{IssueId, OrgId, TeamId, UserId};
use serde_json::Value;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RespRow {
    id: Uuid,
    team_id: Uuid,
    action: String,
    manual_user_ids: Option<Vec<Uuid>>,
    assignment_index: i32,
    time_schedule_id: Option<Uuid>,
    current_user_id: Option<Uuid>,
    version: i32,
}

const RESP_COLS: &str = "id, team_id, action, manual_user_ids, assignment_index, time_schedule_id, \
    current_user_id, version";

impl RespRow {
    fn into_resp(self) -> AppResult<TriageResponsibility> {
        Ok(TriageResponsibility {
            id: self.id,
            team_id: TeamId(self.team_id),
            action: TriageAction::parse(&self.action)
                .ok_or_else(|| AppError::internal("invalid triage action in database"))?,
            manual_user_ids: self
                .manual_user_ids
                .unwrap_or_default()
                .into_iter()
                .map(UserId)
                .collect(),
            assignment_index: self.assignment_index,
            time_schedule_id: self.time_schedule_id,
            current_user_id: self.current_user_id.map(UserId),
            version: self.version,
        })
    }
}

#[derive(sqlx::FromRow)]
struct EntryRow {
    id: Uuid,
    schedule_id: Uuid,
    user_id: Option<Uuid>,
    user_email: Option<String>,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    overridden_user_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: Uuid,
    team_id: Uuid,
    position: f64,
    name: String,
    filter: Value,
    action: String,
    duration: Option<String>,
    sla_type: String,
    start_mode: String,
}

#[derive(sqlx::FromRow)]
struct RecRow {
    id: Uuid,
    team_id: Uuid,
    template_id: Uuid,
    template_name: String,
    frequency: String,
    interval_count: i32,
    weekday: Option<i32>,
    day_of_month: Option<i32>,
    minute_of_day: i32,
    start_date: NaiveDate,
    paused: bool,
    next_run_at: DateTime<Utc>,
    last_run_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    last_error_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
}

impl From<RecRow> for RecurringIssue {
    fn from(r: RecRow) -> Self {
        Self {
            id: r.id,
            team_id: TeamId(r.team_id),
            template_id: r.template_id,
            template_name: r.template_name,
            frequency: r.frequency,
            interval_count: r.interval_count,
            weekday: r.weekday,
            day_of_month: r.day_of_month,
            minute_of_day: r.minute_of_day,
            start_date: r.start_date,
            paused: r.paused,
            next_run_at: r.next_run_at,
            last_run_at: r.last_run_at,
            last_error: r.last_error,
            last_error_at: r.last_error_at,
            version: r.version,
            created_at: r.created_at,
        }
    }
}

const REC_SELECT: &str = "SELECT r.id, r.team_id, r.template_id, t.name AS template_name, r.frequency, \
    r.interval_count, r.weekday, r.day_of_month, r.minute_of_day, r.start_date, r.paused, \
    r.next_run_at, r.last_run_at, r.last_error, r.last_error_at, r.version, r.created_at \
    FROM recurring_issue r JOIN template t ON t.organization_id = r.organization_id AND t.id = r.template_id";

#[async_trait]
impl TriageRepo for PgTx {
    async fn get_responsibility(
        &mut self,
        team: TeamId,
    ) -> AppResult<Option<TriageResponsibility>> {
        let row = sqlx::query_as::<_, RespRow>(&format!(
            "SELECT {RESP_COLS} FROM triage_responsibility \
             WHERE organization_id = $1 AND team_id = $2 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(RespRow::into_resp).transpose()
    }

    async fn upsert_responsibility(
        &mut self,
        r: &NewResponsibility,
    ) -> AppResult<TriageResponsibility> {
        let users: Option<Vec<Uuid>> = (!r.manual_user_ids.is_empty())
            .then(|| r.manual_user_ids.iter().map(|u| u.0).collect());
        let row = sqlx::query_as::<_, RespRow>(&format!(
            "INSERT INTO triage_responsibility \
               (id, organization_id, team_id, action, manual_user_ids, time_schedule_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (team_id) DO UPDATE SET action = EXCLUDED.action, \
               manual_user_ids = EXCLUDED.manual_user_ids, time_schedule_id = EXCLUDED.time_schedule_id, \
               assignment_index = 0, current_user_id = NULL, archived_at = NULL, \
               version = triage_responsibility.version + 1, updated_at = EXCLUDED.updated_at \
             WHERE triage_responsibility.organization_id = EXCLUDED.organization_id \
             RETURNING {RESP_COLS}"
        ))
        .bind(r.id)
        .bind(self.org.0)
        .bind(r.team_id.0)
        .bind(r.action.as_str())
        .bind(users)
        .bind(r.time_schedule_id)
        .bind(r.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.into_resp()
    }

    async fn delete_responsibility(&mut self, team: TeamId) -> AppResult<bool> {
        let n = sqlx::query(
            "DELETE FROM triage_responsibility WHERE organization_id = $1 AND team_id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(n > 0)
    }

    async fn set_current_user(
        &mut self,
        team: TeamId,
        user: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE triage_responsibility SET current_user_id = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND team_id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(user.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn advance_assignment_index(
        &mut self,
        team: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<i32> {
        let old: Option<i32> = sqlx::query_scalar(
            "UPDATE triage_responsibility SET assignment_index = assignment_index + 1, \
             version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND team_id = $2 RETURNING assignment_index - 1",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(old.unwrap_or(0))
    }

    async fn insert_schedule(&mut self, id: Uuid, name: &str, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO time_schedule (id, organization_id, name, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(id)
        .bind(self.org.0)
        .bind(name)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_schedule(&mut self, id: Uuid) -> AppResult<Option<TimeSchedule>> {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT id, name FROM time_schedule \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(id, name)| TimeSchedule { id, name }))
    }

    async fn insert_schedule_entry(
        &mut self,
        e: &NewScheduleEntry,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO time_schedule_entry \
               (id, organization_id, schedule_id, user_id, user_email, starts_at, ends_at, \
                overridden_user_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
        )
        .bind(e.id)
        .bind(self.org.0)
        .bind(e.schedule_id)
        .bind(e.user_id.map(|u| u.0))
        .bind(&e.user_email)
        .bind(e.starts_at)
        .bind(e.ends_at)
        .bind(e.overridden_user_id.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn schedule_entries(
        &mut self,
        schedule: Uuid,
        ends_after: DateTime<Utc>,
        user: Option<UserId>,
        limit: i64,
    ) -> AppResult<Vec<ScheduleEntry>> {
        let rows = sqlx::query_as::<_, EntryRow>(
            "SELECT id, schedule_id, user_id, user_email, starts_at, ends_at, overridden_user_id \
             FROM time_schedule_entry \
             WHERE organization_id = $1 AND schedule_id = $2 AND archived_at IS NULL AND ends_at > $3 \
               AND ($4::uuid IS NULL OR user_id = $4) \
             ORDER BY starts_at, id LIMIT $5",
        )
        .bind(self.org.0)
        .bind(schedule)
        .bind(ends_after)
        .bind(user.map(|u| u.0))
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| ScheduleEntry {
                id: r.id,
                schedule_id: r.schedule_id,
                user_id: r.user_id.map(UserId),
                user_email: r.user_email,
                starts_at: r.starts_at,
                ends_at: r.ends_at,
                overridden_user_id: r.overridden_user_id.map(UserId),
            })
            .collect())
    }

    async fn responsibilities_with_schedule(&mut self) -> AppResult<Vec<TeamRef>> {
        // Bewusst mandantenübergreifend; nur über `Store::begin_system`.
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT r.organization_id, r.team_id FROM triage_responsibility r \
             JOIN team t ON t.id = r.team_id \
             WHERE r.time_schedule_id IS NOT NULL AND r.archived_at IS NULL \
               AND t.triage_enabled AND NOT t.trashed ORDER BY r.organization_id, r.team_id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(refs(rows))
    }

    async fn issue_automation(&mut self, issue: IssueId) -> AppResult<IssueAutomation> {
        let row: Option<(
            bool,
            Option<Uuid>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT auto_closed_by_parent_closing, sla_rule_id, sla_set_notified_at, \
                    sla_high_risk_notified_at, sla_breached_notified_at \
             FROM issue WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(
            row.map_or_else(IssueAutomation::default, |r| IssueAutomation {
                auto_closed_by_parent_closing: r.0,
                sla_rule_id: r.1,
                sla_set_notified_at: r.2,
                sla_high_risk_notified_at: r.3,
                sla_breached_notified_at: r.4,
            }),
        )
    }

    async fn set_closed_by_parent(&mut self, issue: IssueId, value: bool) -> AppResult<()> {
        sqlx::query(
            "UPDATE issue SET auto_closed_by_parent_closing = $3 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(value)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_sla_rule(&mut self, issue: IssueId, rule: Option<Uuid>) -> AppResult<()> {
        sqlx::query(
            "UPDATE issue SET sla_rule_id = $3, sla_set_notified_at = NULL, \
             sla_high_risk_notified_at = NULL, sla_breached_notified_at = NULL \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(rule)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn mark_sla(
        &mut self,
        issue: IssueId,
        marker: SlaMarker,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let column = match marker {
            SlaMarker::Set => "sla_set_notified_at",
            SlaMarker::HighRisk => "sla_high_risk_notified_at",
            SlaMarker::Breached => "sla_breached_notified_at",
        };
        sqlx::query(&format!(
            "UPDATE issue SET {column} = $3 WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_sla_thresholds(
        &mut self,
        issue: IssueId,
        medium: DateTime<Utc>,
        high: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE issue SET sla_medium_risk_at = $3, sla_high_risk_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(medium)
        .bind(high)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn archive_issue(
        &mut self,
        issue: IssueId,
        auto: bool,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE issue SET archived_at = $3, auto_archived_at = CASE WHEN $4 THEN $3 ELSE auto_archived_at END, \
             version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(now)
        .bind(auto)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn move_attachments(&mut self, from: IssueId, to: IssueId) -> AppResult<u64> {
        let n = sqlx::query(
            "UPDATE attachment a SET original_issue_id = a.issue_id, issue_id = $3, \
               version = a.version + 1, updated_at = now() \
             WHERE a.organization_id = $1 AND a.issue_id = $2 AND a.archived_at IS NULL \
               AND NOT EXISTS (SELECT 1 FROM attachment b WHERE b.issue_id = $3 AND b.url = a.url)",
        )
        .bind(self.org.0)
        .bind(from.0)
        .bind(to.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(n)
    }

    async fn delete_issue_leftovers(&mut self, issue: IssueId) -> AppResult<()> {
        // `document_content.owner_id` ist polymorph ohne Fremdschlüssel (Schema 0016).
        sqlx::query(
            "DELETE FROM document_content_history WHERE organization_id = $1 AND document_content_id IN \
               (SELECT id FROM document_content WHERE organization_id = $1 AND owner_type = 'issue' AND owner_id = $2)",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "DELETE FROM document_content WHERE organization_id = $1 AND owner_type = 'issue' AND owner_id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn team_sla(&mut self, team: TeamId) -> AppResult<TeamSla> {
        let row: Option<(bool, String)> = sqlx::query_as(
            "SELECT sla_enabled, sla_work_week FROM team WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(match row {
            Some((enabled, week)) => TeamSla {
                enabled,
                sun_thu: week == "sunToThu",
            },
            None => TeamSla {
                enabled: false,
                sun_thu: false,
            },
        })
    }

    async fn sla_rules(&mut self, team: TeamId) -> AppResult<Vec<SlaRule>> {
        let rows = sqlx::query_as::<_, RuleRow>(
            "SELECT id, team_id, position, name, filter, action, duration, sla_type, start_mode \
             FROM sla_rule WHERE organization_id = $1 AND team_id = $2 AND enabled AND archived_at IS NULL \
             ORDER BY position, created_at, id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|r| {
                Ok(SlaRule {
                    id: r.id,
                    team_id: TeamId(r.team_id),
                    position: r.position,
                    name: r.name,
                    filter: r.filter,
                    removes_sla: r.action == "remove",
                    duration: r.duration,
                    sla_type: r
                        .sla_type
                        .parse::<SlaType>()
                        .map_err(|_| AppError::internal("invalid sla type in database"))?,
                    start_mode: r.start_mode,
                })
            })
            .collect()
    }

    async fn insert_sla_rule(&mut self, r: &NewSlaRule) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO sla_rule (id, organization_id, team_id, position, name, filter, action, \
             duration, sla_type, start_mode, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(r.id)
        .bind(self.org.0)
        .bind(r.team_id.0)
        .bind(r.position)
        .bind(&r.name)
        .bind(&r.filter)
        .bind(if r.removes_sla { "remove" } else { "set" })
        .bind(&r.duration)
        .bind(r.sla_type.as_str())
        .bind(&r.start_mode)
        .bind(r.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn save_sla_rule(&mut self, r: &SlaRule, now: DateTime<Utc>) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE sla_rule SET position = $4, name = $5, filter = $6, action = $7, duration = $8, \
             sla_type = $9, start_mode = $10, version = version + 1, updated_at = $11 \
             WHERE organization_id = $1 AND team_id = $2 AND id = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(r.team_id.0)
        .bind(r.id)
        .bind(r.position)
        .bind(&r.name)
        .bind(&r.filter)
        .bind(if r.removes_sla { "remove" } else { "set" })
        .bind(&r.duration)
        .bind(r.sla_type.as_str())
        .bind(&r.start_mode)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_sla_rule(&mut self, team: TeamId, id: Uuid) -> AppResult<bool> {
        let res = sqlx::query(
            "DELETE FROM sla_rule WHERE organization_id = $1 AND team_id = $2 AND id = $3",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn set_team_sla(
        &mut self,
        team: TeamId,
        enabled: Option<bool>,
        sun_thu: Option<bool>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET sla_enabled = COALESCE($3, sla_enabled), \
             sla_work_week = CASE WHEN $4::boolean IS NULL THEN sla_work_week \
               WHEN $4 THEN 'sunToThu' ELSE 'monToFri' END, \
             version = version + 1, updated_at = $5 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(enabled)
        .bind(sun_thu)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn insert_recurring(&mut self, n: &NewRecurringIssue) -> AppResult<()> {
        let r = &n.rec;
        sqlx::query(
            "INSERT INTO recurring_issue (id, organization_id, team_id, template_id, frequency, \
             interval_count, weekday, day_of_month, minute_of_day, start_date, paused, next_run_at, \
             creator_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)",
        )
        .bind(r.id)
        .bind(self.org.0)
        .bind(r.team_id.0)
        .bind(r.template_id)
        .bind(&r.frequency)
        .bind(r.interval_count)
        .bind(r.weekday)
        .bind(r.day_of_month)
        .bind(r.minute_of_day)
        .bind(r.start_date)
        .bind(r.paused)
        .bind(r.next_run_at)
        .bind(n.creator_id.map(|u| u.0))
        .bind(n.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list_recurring(&mut self, team: TeamId) -> AppResult<Vec<RecurringIssue>> {
        let rows = sqlx::query_as::<_, RecRow>(&format!(
            "{REC_SELECT} WHERE r.organization_id = $1 AND r.team_id = $2 ORDER BY r.created_at, r.id"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_recurring(&mut self, id: Uuid) -> AppResult<Option<RecurringIssue>> {
        let row = sqlx::query_as::<_, RecRow>(&format!(
            "{REC_SELECT} WHERE r.organization_id = $1 AND r.id = $2"
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn save_recurring(&mut self, r: &RecurringIssue, now: DateTime<Utc>) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE recurring_issue SET template_id = $3, frequency = $4, interval_count = $5, \
             weekday = $6, day_of_month = $7, minute_of_day = $8, start_date = $9, paused = $10, \
             next_run_at = $11, version = version + 1, updated_at = $12 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(r.id)
        .bind(r.template_id)
        .bind(&r.frequency)
        .bind(r.interval_count)
        .bind(r.weekday)
        .bind(r.day_of_month)
        .bind(r.minute_of_day)
        .bind(r.start_date)
        .bind(r.paused)
        .bind(r.next_run_at)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_recurring(&mut self, id: Uuid) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM recurring_issue WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn due_recurring(
        &mut self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<RecurringIssue>> {
        let rows = sqlx::query_as::<_, RecRow>(&format!(
            "{REC_SELECT} WHERE r.organization_id = $1 AND NOT r.paused AND r.next_run_at <= $2 \
             ORDER BY r.next_run_at, r.id LIMIT $3 FOR UPDATE OF r SKIP LOCKED"
        ))
        .bind(self.org.0)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn claim_recurring_run(
        &mut self,
        id: Uuid,
        occurrence: DateTime<Utc>,
        issue: Option<IssueId>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO recurring_issue_run (recurring_issue_id, occurrence_at, issue_id) \
             SELECT r.id, $3, $4 FROM recurring_issue r WHERE r.organization_id = $1 AND r.id = $2 \
             ON CONFLICT DO NOTHING",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(occurrence)
        .bind(issue.map(|i| i.0))
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn finish_recurring_run(
        &mut self,
        id: Uuid,
        ran_at: DateTime<Utc>,
        next: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE recurring_issue SET last_run_at = $3, next_run_at = $4, last_error = NULL, \
             last_error_at = NULL, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(ran_at)
        .bind(next)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn skip_recurring_run(
        &mut self,
        id: Uuid,
        next: DateTime<Utc>,
        error: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE recurring_issue SET next_run_at = $3, last_error = left($4, 500), \
             last_error_at = $5, version = version + 1, updated_at = $5 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(next)
        .bind(error)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn teams_with_tight_sort_order(
        &mut self,
        min_gap: f64,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<TeamRef>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT o.organization_id, o.team_id FROM ( \
               SELECT i.organization_id, i.team_id, \
                      i.sort_order - lag(i.sort_order) OVER (PARTITION BY i.team_id \
                                                             ORDER BY i.sort_order, i.id) AS gap \
               FROM issue i WHERE i.archived_at IS NULL AND NOT i.trashed) o \
             JOIN team t ON t.organization_id = o.organization_id AND t.id = o.team_id \
             WHERE o.gap < $1 AND NOT t.trashed \
               AND (t.import_locked_until IS NULL OR t.import_locked_until <= $2) \
             ORDER BY 1, 2",
        )
        .bind(min_gap)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(refs(rows))
    }

    async fn renumber_team_sort_order(&mut self, team: TeamId) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "WITH ranked AS ( \
               SELECT id, sort_order AS old, \
                      min(sort_order) OVER () + (row_number() OVER (ORDER BY sort_order, id) - 1)::float8 AS new \
               FROM issue WHERE organization_id = $1 AND team_id = $2 AND archived_at IS NULL AND NOT trashed) \
             UPDATE issue i SET sort_order = r.new, version = i.version + 1 \
             FROM ranked r WHERE i.id = r.id AND r.new <> r.old \
             RETURNING i.id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn count_recurring_for_template(&mut self, template: Uuid) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM recurring_issue WHERE organization_id = $1 AND template_id = $2",
        )
        .bind(self.org.0)
        .bind(template)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn pause_recurring_run(
        &mut self,
        id: Uuid,
        error: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE recurring_issue SET paused = true, last_error = left($3, 500), \
             last_error_at = $4, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(error)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn orgs_with_recurring_due(&mut self, now: DateTime<Utc>) -> AppResult<Vec<OrgId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT organization_id FROM recurring_issue \
             WHERE NOT paused AND next_run_at <= $1 ORDER BY organization_id",
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(OrgId).collect())
    }

    async fn teams_with_auto_close(&mut self, now: DateTime<Utc>) -> AppResult<Vec<TeamRef>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT organization_id, id FROM team WHERE auto_close_period IS NOT NULL AND NOT trashed \
             AND retired_at IS NULL AND archived_at IS NULL \
             AND (import_locked_until IS NULL OR import_locked_until <= $1) ORDER BY organization_id, id",
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(refs(rows))
    }

    async fn teams_with_auto_archive(&mut self, now: DateTime<Utc>) -> AppResult<Vec<TeamRef>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT organization_id, id FROM team WHERE NOT trashed AND archived_at IS NULL \
             AND (import_locked_until IS NULL OR import_locked_until <= $1) ORDER BY organization_id, id",
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(refs(rows))
    }

    async fn orgs_with_trashed_issues(&mut self) -> AppResult<Vec<OrgId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT organization_id FROM issue WHERE trashed ORDER BY organization_id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(OrgId).collect())
    }

    async fn orgs_with_sla(&mut self) -> AppResult<Vec<OrgId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT organization_id FROM ( \
               SELECT organization_id FROM team WHERE sla_enabled AND NOT trashed \
               UNION ALL \
               SELECT organization_id FROM issue WHERE sla_breaches_at IS NOT NULL \
                 AND state_type NOT IN ('completed', 'canceled', 'duplicate') AND archived_at IS NULL AND NOT trashed \
             ) o ORDER BY organization_id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(OrgId).collect())
    }

    async fn orgs_with_due_snooze(&mut self, now: DateTime<Utc>) -> AppResult<Vec<OrgId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT organization_id FROM issue WHERE state_type = 'triage' \
             AND snoozed_until_at IS NOT NULL AND snoozed_until_at <= $1 ORDER BY organization_id",
        )
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(OrgId).collect())
    }

    async fn auto_close_candidates(
        &mut self,
        team: TeamId,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
        after: Option<IssueId>,
        limit: i64,
    ) -> AppResult<Vec<AutoCloseCandidate>> {
        let rows: Vec<(Uuid, bool, bool)> = sqlx::query_as(
            "SELECT i.id, \
               EXISTS (SELECT 1 FROM cycle c WHERE c.id = i.cycle_id AND c.completed_at IS NULL \
                       AND c.starts_at <= $4 AND $4 < c.ends_at), \
               (i.project_id IS NOT NULL AND EXISTS (SELECT 1 FROM project p JOIN project_status ps ON ps.id = p.status_id \
                       WHERE p.id = i.project_id AND ps.type NOT IN ('completed', 'canceled'))) \
             FROM issue i \
             WHERE i.organization_id = $1 AND i.team_id = $2 AND i.state_type IN ('triage', 'backlog', 'unstarted') \
               AND i.archived_at IS NULL AND NOT i.trashed AND i.updated_at < $3 \
               AND ($5::uuid IS NULL OR i.id > $5) \
             ORDER BY i.id LIMIT $6",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(cutoff)
        .bind(now)
        .bind(after.map(|i| i.0))
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, c, p)| AutoCloseCandidate {
                issue_id: IssueId(id),
                in_active_cycle: c,
                in_unfinished_project: p,
            })
            .collect())
    }

    async fn auto_close_info(
        &mut self,
        issue: IssueId,
        now: DateTime<Utc>,
    ) -> AppResult<AutoCloseCandidate> {
        let row: Option<(bool, bool)> = sqlx::query_as(
            "SELECT \
               EXISTS (SELECT 1 FROM cycle c WHERE c.id = i.cycle_id AND c.completed_at IS NULL \
                       AND c.starts_at <= $3 AND $3 < c.ends_at), \
               (i.project_id IS NOT NULL AND EXISTS (SELECT 1 FROM project p JOIN project_status ps ON ps.id = p.status_id \
                       WHERE p.id = i.project_id AND ps.type NOT IN ('completed', 'canceled'))) \
             FROM issue i WHERE i.organization_id = $1 AND i.id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let (c, p) = row.unwrap_or((false, false));
        Ok(AutoCloseCandidate {
            issue_id: issue,
            in_active_cycle: c,
            in_unfinished_project: p,
        })
    }

    async fn auto_archive_candidates(
        &mut self,
        team: TeamId,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(&auto_archive_sql(false))
            .bind(self.org.0)
            .bind(team.0)
            .bind(cutoff)
            .bind(limit)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn auto_archive_unwarned_candidates(
        &mut self,
        team: TeamId,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(&auto_archive_sql(true))
            .bind(self.org.0)
            .bind(team.0)
            .bind(cutoff)
            .bind(limit)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn purge_trashed_projects(
        &mut self,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<lineal_domain::ids::ProjectId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM project WHERE organization_id = $1 AND trashed AND trashed_at IS NOT NULL \
             AND trashed_at <= $2 ORDER BY trashed_at, id LIMIT $3",
        )
        .bind(self.org.0)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query(
            "UPDATE issue SET project_id = NULL, project_milestone_id = NULL, added_to_project_at = NULL, \
               version = version + 1, updated_at = now() \
             WHERE organization_id = $1 AND project_id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(&ids)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query("DELETE FROM project WHERE organization_id = $1 AND id = ANY($2)")
            .bind(self.org.0)
            .bind(&ids)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(ids.into_iter().map(lineal_domain::ids::ProjectId).collect())
    }

    async fn purge_candidates(
        &mut self,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND trashed AND trashed_at IS NOT NULL \
             AND trashed_at <= $2 ORDER BY trashed_at, id LIMIT $3",
        )
        .bind(self.org.0)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn sla_due(&mut self, now: DateTime<Utc>, limit: i64) -> AppResult<Vec<IssueId>> {
        // Eine Meldemarke gilt nur, wenn sie nicht älter ist als der Zeitpunkt, den sie meldet
        // (ein verlängertes oder neu gesetztes SLA meldet erneut).
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND sla_breaches_at IS NOT NULL \
               AND state_type NOT IN ('completed', 'canceled', 'duplicate') AND archived_at IS NULL AND NOT trashed \
               AND (sla_medium_risk_at IS NULL OR sla_high_risk_at IS NULL \
                    OR sla_set_notified_at IS NULL OR sla_set_notified_at < sla_started_at \
                    OR (sla_high_risk_at <= $2 AND (sla_high_risk_notified_at IS NULL OR sla_high_risk_notified_at < sla_high_risk_at)) \
                    OR (sla_breaches_at <= $2 AND (sla_breached_notified_at IS NULL OR sla_breached_notified_at < sla_breaches_at))) \
             ORDER BY id LIMIT $3",
        )
        .bind(self.org.0)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn sla_rule_candidates(
        &mut self,
        since: DateTime<Utc>,
        after: Option<IssueId>,
        limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT i.id FROM issue i JOIN team t ON t.id = i.team_id \
             WHERE i.organization_id = $1 AND t.sla_enabled \
               AND i.state_type NOT IN ('completed', 'canceled', 'duplicate') \
               AND i.archived_at IS NULL AND NOT i.trashed AND i.updated_at >= $2 \
               AND ($3::uuid IS NULL OR i.id > $3) \
             ORDER BY i.id LIMIT $4",
        )
        .bind(self.org.0)
        .bind(since)
        .bind(after.map(|i| i.0))
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn due_snoozed(&mut self, now: DateTime<Utc>, limit: i64) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND state_type = 'triage' \
             AND snoozed_until_at IS NOT NULL AND snoozed_until_at <= $2 ORDER BY id LIMIT $3",
        )
        .bind(self.org.0)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn triage_issues(
        &mut self,
        team: TeamId,
        now: DateTime<Utc>,
        include_snoozed: bool,
        limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2 AND state_type = 'triage' \
               AND archived_at IS NULL AND NOT trashed \
               AND ($4 OR snoozed_until_at IS NULL OR snoozed_until_at <= $3) \
             ORDER BY started_triage_at NULLS LAST, id LIMIT $5",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(now)
        .bind(include_snoozed)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }
}

/// SQL der Auto-Archive-Kandidaten; `unwarned` schließt Issues mit bereits erzeugter Vorwarnung aus.
fn auto_archive_sql(unwarned: bool) -> String {
    let warned = if unwarned {
        " AND NOT EXISTS (SELECT 1 FROM notification w WHERE w.issue_id = i.id \
           AND w.type = 'issueAutoArchiveWarning' AND w.created_at >= i.updated_at)"
    } else {
        ""
    };
    format!(
        "SELECT i.id FROM issue i \
             WHERE i.organization_id = $1 AND i.team_id = $2 \
               AND i.state_type IN ('completed', 'canceled', 'duplicate') \
               AND i.archived_at IS NULL AND NOT i.trashed \
               AND coalesce(i.completed_at, i.canceled_at, i.state_entered_at) <= $3 \
               AND i.updated_at <= $3 \
               AND NOT EXISTS (SELECT 1 FROM issue p WHERE p.id = i.parent_id \
                               AND p.state_type IN ('triage', 'backlog', 'unstarted', 'started')) \
               AND NOT EXISTS (SELECT 1 FROM issue c WHERE c.parent_id = i.id AND NOT c.trashed \
                               AND c.state_type IN ('triage', 'backlog', 'unstarted', 'started')) \
               AND (i.cycle_id IS NULL OR EXISTS (SELECT 1 FROM cycle c WHERE c.id = i.cycle_id \
                               AND c.completed_at IS NOT NULL AND c.completed_at <= $3)) \
               AND (i.project_id IS NULL OR EXISTS (SELECT 1 FROM project p JOIN project_status ps ON ps.id = p.status_id \
                               WHERE p.id = i.project_id AND ps.type IN ('completed', 'canceled') \
                               AND coalesce(p.completed_at, p.canceled_at, p.updated_at) <= $3)){warned} ORDER BY i.id LIMIT $4"
    )
}

fn refs(rows: Vec<(Uuid, Uuid)>) -> Vec<TeamRef> {
    rows.into_iter()
        .map(|(o, t)| TeamRef {
            org_id: OrgId(o),
            team_id: TeamId(t),
        })
        .collect()
}
