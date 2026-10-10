//! Repository-Port `OverviewRepo`: Kennzahlen der firmenweiten Sicht (WP11, docs/14).
//!
//! Alle Abfragen bekommen die Teams, über die gerechnet werden darf, **bereits auf sichtbare
//! Teams geschnitten** (`teams`). Aggregate laufen ausschließlich über diese Teams; ein privates
//! Team ohne Mitgliedschaft taucht weder als Zeile noch in Summen auf.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::ids::{CycleId, InitiativeId, IssueId, ProjectId, TeamId, UserId};

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Gemeinsame Parameter der Kennzahlen.
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewQuery {
    /// Teams, über die gerechnet wird (sichtbar!).
    pub teams: Vec<TeamId>,
    pub now: DateTime<Utc>,
    /// „Heute“ für Fälligkeiten.
    pub today: NaiveDate,
    /// Berichtszeitraum für „erledigt“ (7 oder 30 Tage).
    pub period_days: i64,
}

/// Rohzahlen eines Teams (nur die eigenen Issues, ohne Sub-Teams).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TeamIssueStats {
    pub team_id: Option<TeamId>,
    /// Kategorien `triage`, `backlog`, `unstarted`, `started`.
    pub open: i64,
    pub started: i64,
    pub triage: i64,
    pub triage_oldest_at: Option<DateTime<Utc>>,
    /// `completedAt` im Berichtszeitraum.
    pub completed: i64,
    pub completed_points: f64,
    /// `dueDate < heute` und offen.
    pub overdue: i64,
    /// `slaHighRiskAt <= now`, noch nicht verletzt, offen.
    pub sla_at_risk: i64,
    /// `slaBreachesAt <= now`, offen.
    pub sla_breached: i64,
}

/// Durchsatz eines Teams in einer Woche (`week_offset` 0 = jüngste 7 Tage).
#[derive(Debug, Clone, PartialEq)]
pub struct WeekThroughput {
    pub team_id: TeamId,
    pub week_offset: i32,
    pub issues: i64,
    pub points: f64,
}

/// Ein (Team, Projekt)-Paar mit Health für „Projekte nach Health“.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectHealthRow {
    pub team_id: TeamId,
    pub project_id: ProjectId,
    pub health: Option<String>,
}

/// Initiative für die Übersicht.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeRow {
    pub id: InitiativeId,
    pub number: i32,
    pub name: String,
    pub status: String,
    pub health: Option<String>,
    /// Sichtbare Projekte der Initiative.
    pub project_count: i64,
    pub completed_project_count: i64,
}

/// Überfälliges oder SLA-gefährdetes Issue.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRiskRow {
    pub id: IssueId,
    pub identifier: String,
    pub title: String,
    pub team_id: TeamId,
    pub team_key: String,
    pub due_date: Option<NaiveDate>,
    pub sla_breaches_at: Option<DateTime<Utc>>,
    /// `overdue`, `slaAtRisk`, `slaBreached`.
    pub reasons: Vec<String>,
}

/// Welcher Cycle der Auslastung zugrunde liegt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadCycle {
    Current,
    Next,
}

/// Auslastung: offene Issues und Punkte eines Users in einem Team.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadRow {
    pub user_id: UserId,
    pub user_name: String,
    pub user_display_name: String,
    pub team_id: TeamId,
    pub open_issues: i64,
    pub points: f64,
}

/// Aktiver Cycle eines Teams (Roh).
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCycleRow {
    pub team_id: TeamId,
    pub cycle_id: CycleId,
    pub number: i32,
    pub name: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Issues, die nach dem Start hinzugefügt wurden (Scope-Änderung).
    pub added_after_start: i64,
}

/// Persistenzzugriff des Bereichs `overview`.
#[async_trait]
pub trait OverviewRepo: Send {
    /// Zahlen je Team (`GROUP BY team_id`); Teams ohne Issues fehlen.
    async fn issue_stats(&mut self, _q: &OverviewQuery) -> AppResult<Vec<TeamIssueStats>> {
        unimplemented_repo()
    }
    /// Erledigte Issues und Punkte der letzten 6 Wochen je Team.
    async fn throughput(&mut self, _q: &OverviewQuery) -> AppResult<Vec<WeekThroughput>> {
        unimplemented_repo()
    }
    /// Nicht abgeschlossene, nicht gelöschte Projekte je beteiligtem Team aus `q.teams`.
    async fn project_health(&mut self, _q: &OverviewQuery) -> AppResult<Vec<ProjectHealthRow>> {
        unimplemented_repo()
    }
    /// Sichtbare Initiativen (`visible` = sichtbare Teams; nicht für Gäste aufrufen).
    async fn initiatives(
        &mut self,
        _visible: &[TeamId],
        _limit: i64,
    ) -> AppResult<Vec<InitiativeRow>> {
        unimplemented_repo()
    }
    /// Überfällige/SLA-gefährdete offene Issues, dringendste zuerst.
    async fn at_risk_issues(&mut self, _q: &OverviewQuery, _limit: i64) -> AppResult<Vec<AtRiskRow>> {
        unimplemented_repo()
    }
    /// Auslastung je (User, Team) im aktuellen oder nächsten Cycle.
    async fn workload(
        &mut self,
        _q: &OverviewQuery,
        _cycle: WorkloadCycle,
    ) -> AppResult<Vec<WorkloadRow>> {
        unimplemented_repo()
    }
    /// Aktive Cycles der Teams aus `q.teams`.
    async fn active_cycles(&mut self, _q: &OverviewQuery) -> AppResult<Vec<ActiveCycleRow>> {
        unimplemented_repo()
    }
}
