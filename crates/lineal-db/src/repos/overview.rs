//! `OverviewRepo` für `PgTx` (WP11, docs/14 §3): SQL-Aggregation `GROUP BY team_id, state_type`
//! über die Issue-Tabelle. Jede Abfrage trägt `organization_id` und `team_id = ANY(teams)` mit
//! den bereits auf sichtbare Teams geschnittenen Teams des Aufrufers.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::OverviewRepo;
use lineal_app::ports::repos::overview::{
    ActiveCycleRow, AtRiskRow, InitiativeRow, OverviewQuery, ProjectHealthRow, TeamIssueStats,
    WeekThroughput, WorkloadCycle, WorkloadRow,
};
use lineal_domain::ids::{CycleId, InitiativeId, IssueId, ProjectId, TeamId, UserId};
use uuid::Uuid;

fn ids(teams: &[TeamId]) -> Vec<Uuid> {
    teams.iter().map(|t| t.0).collect()
}

/// Punkte eines Issues nach den Einstellungen seines Teams (docs/01 §6.3).
const POINTS: &str =
    "(CASE WHEN t.issue_estimation_type = 'notUsed' THEN 1 ELSE COALESCE(i.estimate, t.default_issue_estimate) END)::float8";
const OPEN: &str = "i.state_type IN ('triage','backlog','unstarted','started')";

#[async_trait]
impl OverviewRepo for PgTx {
    async fn issue_stats(&mut self, q: &OverviewQuery) -> AppResult<Vec<TeamIssueStats>> {
        let sql = format!(
            "SELECT i.team_id, \
               count(*) FILTER (WHERE {OPEN}) AS open, \
               count(*) FILTER (WHERE i.state_type = 'started') AS started, \
               count(*) FILTER (WHERE i.state_type = 'triage') AS triage, \
               min(i.created_at) FILTER (WHERE i.state_type = 'triage') AS triage_oldest, \
               count(*) FILTER (WHERE i.state_type = 'completed' AND i.completed_at >= $3 - make_interval(days => $4::int)) AS completed, \
               COALESCE(sum({POINTS}) FILTER (WHERE i.state_type = 'completed' AND i.completed_at >= $3 - make_interval(days => $4::int)), 0)::float8 AS completed_points, \
               count(*) FILTER (WHERE {OPEN} AND i.due_date < $5) AS overdue, \
               count(*) FILTER (WHERE {OPEN} AND i.sla_high_risk_at <= $3 AND (i.sla_breaches_at IS NULL OR i.sla_breaches_at > $3)) AS sla_at_risk, \
               count(*) FILTER (WHERE {OPEN} AND i.sla_breaches_at <= $3) AS sla_breached \
             FROM issue i JOIN team t ON t.id = i.team_id \
             WHERE i.organization_id = $1 AND NOT i.trashed AND i.archived_at IS NULL AND i.team_id = ANY($2) \
             GROUP BY i.team_id"
        );
        let rows: Vec<(
            Uuid,
            i64,
            i64,
            i64,
            Option<DateTime<Utc>>,
            i64,
            f64,
            i64,
            i64,
            i64,
        )> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(ids(&q.teams))
            .bind(q.now)
            .bind(q.period_days as i32)
            .bind(q.today)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| TeamIssueStats {
                team_id: Some(TeamId(r.0)),
                open: r.1,
                started: r.2,
                triage: r.3,
                triage_oldest_at: r.4,
                completed: r.5,
                completed_points: r.6,
                overdue: r.7,
                sla_at_risk: r.8,
                sla_breached: r.9,
            })
            .collect())
    }

    async fn throughput(&mut self, q: &OverviewQuery) -> AppResult<Vec<WeekThroughput>> {
        let sql = format!(
            "SELECT i.team_id, floor(extract(epoch FROM ($3 - i.completed_at)) / 604800)::int AS week, \
               count(*), COALESCE(sum({POINTS}), 0)::float8 \
             FROM issue i JOIN team t ON t.id = i.team_id \
             WHERE i.organization_id = $1 AND NOT i.trashed AND i.team_id = ANY($2) \
               AND i.state_type = 'completed' AND i.completed_at IS NOT NULL \
               AND i.completed_at <= $3 AND i.completed_at > $3 - interval '42 days' \
             GROUP BY 1, 2"
        );
        let rows: Vec<(Uuid, i32, i64, f64)> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(ids(&q.teams))
            .bind(q.now)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| WeekThroughput {
                team_id: TeamId(r.0),
                week_offset: r.1,
                issues: r.2,
                points: r.3,
            })
            .collect())
    }

    async fn project_health(&mut self, q: &OverviewQuery) -> AppResult<Vec<ProjectHealthRow>> {
        let rows: Vec<(Uuid, Uuid, Option<String>)> = sqlx::query_as(
            "SELECT pt.team_id, p.id, p.health FROM project p \
             JOIN project_team pt ON pt.project_id = p.id \
             JOIN project_status ps ON ps.id = p.status_id \
             WHERE p.organization_id = $1 AND NOT p.trashed AND p.archived_at IS NULL \
               AND ps.type NOT IN ('completed','canceled') AND pt.team_id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(ids(&q.teams))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| ProjectHealthRow {
                team_id: TeamId(r.0),
                project_id: ProjectId(r.1),
                health: r.2,
            })
            .collect())
    }

    async fn initiatives(&mut self, visible: &[TeamId], limit: i64) -> AppResult<Vec<InitiativeRow>> {
        let rows: Vec<(Uuid, i32, String, String, Option<String>, i64, i64)> = sqlx::query_as(
            "SELECT n.id, n.number, n.name, n.status, n.health, \
               (SELECT count(*) FROM initiative_to_project ip JOIN project p ON p.id = ip.project_id \
                 WHERE ip.initiative_id = n.id AND NOT p.trashed \
                   AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY($2))), \
               (SELECT count(*) FROM initiative_to_project ip JOIN project p ON p.id = ip.project_id \
                 JOIN project_status ps ON ps.id = p.status_id \
                 WHERE ip.initiative_id = n.id AND NOT p.trashed AND ps.type = 'completed' \
                   AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY($2))) \
             FROM initiative n \
             WHERE n.organization_id = $1 AND NOT n.trashed AND n.archived_at IS NULL \
               AND n.status IN ('Proposed','Planned','Active') \
               AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY($2)) \
             ORDER BY n.number LIMIT $3",
        )
        .bind(self.org.0)
        .bind(ids(visible))
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| InitiativeRow {
                id: InitiativeId(r.0),
                number: r.1,
                name: r.2,
                status: r.3,
                health: r.4,
                project_count: r.5,
                completed_project_count: r.6,
            })
            .collect())
    }

    async fn at_risk_issues(&mut self, q: &OverviewQuery, limit: i64) -> AppResult<Vec<AtRiskRow>> {
        let sql = format!(
            "SELECT i.id, i.identifier, i.title, i.team_id, t.key, i.due_date, i.sla_breaches_at, \
               (i.due_date < $4) AS overdue, \
               (i.sla_breaches_at <= $3) AS breached, \
               (i.sla_high_risk_at <= $3 AND (i.sla_breaches_at IS NULL OR i.sla_breaches_at > $3)) AS at_risk \
             FROM issue i JOIN team t ON t.id = i.team_id \
             WHERE i.organization_id = $1 AND NOT i.trashed AND i.archived_at IS NULL \
               AND i.team_id = ANY($2) AND {OPEN} \
               AND (i.due_date < $4 OR i.sla_breaches_at <= $3 OR i.sla_high_risk_at <= $3) \
             ORDER BY (i.sla_breaches_at <= $3) DESC NULLS LAST, i.due_date ASC NULLS LAST, i.id \
             LIMIT $5"
        );
        let rows: Vec<(
            Uuid,
            String,
            String,
            Uuid,
            String,
            Option<NaiveDate>,
            Option<DateTime<Utc>>,
            Option<bool>,
            Option<bool>,
            Option<bool>,
        )> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(ids(&q.teams))
            .bind(q.now)
            .bind(q.today)
            .bind(limit)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let mut reasons = Vec::new();
                if r.7 == Some(true) {
                    reasons.push("overdue".to_owned());
                }
                if r.9 == Some(true) {
                    reasons.push("slaAtRisk".to_owned());
                }
                if r.8 == Some(true) {
                    reasons.push("slaBreached".to_owned());
                }
                AtRiskRow {
                    id: IssueId(r.0),
                    identifier: r.1,
                    title: r.2,
                    team_id: TeamId(r.3),
                    team_key: r.4,
                    due_date: r.5,
                    sla_breaches_at: r.6,
                    reasons,
                }
            })
            .collect())
    }

    async fn workload(&mut self, q: &OverviewQuery, cycle: WorkloadCycle) -> AppResult<Vec<WorkloadRow>> {
        let cycle_cond = match cycle {
            WorkloadCycle::Current => {
                "c.starts_at <= $3 AND $3 < c.ends_at AND c.completed_at IS NULL"
            }
            WorkloadCycle::Next => {
                "c.id = (SELECT n.id FROM cycle n WHERE n.team_id = c.team_id AND n.starts_at > $3 \
                 ORDER BY n.starts_at LIMIT 1)"
            }
        };
        let sql = format!(
            "SELECT u.id, u.name, u.display_name, i.team_id, count(*), COALESCE(sum({POINTS}), 0)::float8 \
             FROM issue i JOIN team t ON t.id = i.team_id \
             JOIN cycle c ON c.id = i.cycle_id AND c.organization_id = i.organization_id \
             JOIN \"user\" u ON u.id = i.assignee_id AND u.organization_id = i.organization_id \
             WHERE i.organization_id = $1 AND NOT i.trashed AND i.archived_at IS NULL \
               AND i.team_id = ANY($2) AND {OPEN} AND {cycle_cond} \
             GROUP BY u.id, u.name, u.display_name, i.team_id"
        );
        let rows: Vec<(Uuid, String, String, Uuid, i64, f64)> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(ids(&q.teams))
            .bind(q.now)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| WorkloadRow {
                user_id: UserId(r.0),
                user_name: r.1,
                user_display_name: r.2,
                team_id: TeamId(r.3),
                open_issues: r.4,
                points: r.5,
            })
            .collect())
    }

    async fn active_cycles(&mut self, q: &OverviewQuery) -> AppResult<Vec<ActiveCycleRow>> {
        let rows: Vec<(Uuid, Uuid, i32, Option<String>, DateTime<Utc>, DateTime<Utc>, i64)> =
            sqlx::query_as(
                "SELECT c.team_id, c.id, c.number, c.name, c.starts_at, c.ends_at, \
                   (SELECT count(*) FROM issue i WHERE i.cycle_id = c.id AND NOT i.trashed \
                      AND i.added_to_cycle_at > c.starts_at) \
                 FROM cycle c WHERE c.organization_id = $1 AND c.team_id = ANY($2) \
                   AND c.archived_at IS NULL AND c.completed_at IS NULL \
                   AND c.starts_at <= $3 AND $3 < c.ends_at ORDER BY c.team_id",
            )
            .bind(self.org.0)
            .bind(ids(&q.teams))
            .bind(q.now)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| ActiveCycleRow {
                team_id: TeamId(r.0),
                cycle_id: CycleId(r.1),
                number: r.2,
                name: r.3,
                starts_at: r.4,
                ends_at: r.5,
                added_after_start: r.6,
            })
            .collect())
    }
}
