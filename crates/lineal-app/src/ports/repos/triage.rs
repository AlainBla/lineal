//! Repository-Port `TriageRepo` (WP12): Triage-Verantwortung und Schichtpläne, Spalten der
//! Automationen am Issue (`autoClosedByParentClosing`, SLA-Regel/-Meldemarken), SLA-Regeln und
//! die Kandidatenabfragen der Jobs (Auto-Close, Auto-Archive, Purge, SLA, Snooze).
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`). Methoden, die als
//! „mandantenübergreifend“ gekennzeichnet sind, sind nur über `Store::begin_system` aufzurufen.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::SlaType;
use lineal_domain::ids::{IssueId, OrgId, TeamId, UserId};
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Aktion der Triage-Verantwortung (docs/03 §10.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageAction {
    Notify,
    Assign,
}

impl TriageAction {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::Assign => "assign",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "notify" => Some(Self::Notify),
            "assign" => Some(Self::Assign),
            _ => None,
        }
    }
}

/// `TriageResponsibility` (höchstens eine je Team).
#[derive(Debug, Clone, PartialEq)]
pub struct TriageResponsibility {
    pub id: Uuid,
    pub team_id: TeamId,
    pub action: TriageAction,
    /// Manuelle Auswahl (Alternative zum Schichtplan).
    pub manual_user_ids: Vec<UserId>,
    pub assignment_index: i32,
    pub time_schedule_id: Option<Uuid>,
    /// Gespeicherter aktueller Verantwortlicher (Rotation-Job).
    pub current_user_id: Option<UserId>,
    pub version: i32,
}

/// Neue oder geänderte Verantwortung (genau eine Quelle).
#[derive(Debug, Clone, PartialEq)]
pub struct NewResponsibility {
    pub id: Uuid,
    pub team_id: TeamId,
    pub action: TriageAction,
    pub manual_user_ids: Vec<UserId>,
    pub time_schedule_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

/// Schichtplan (`TimeSchedule`).
#[derive(Debug, Clone, PartialEq)]
pub struct TimeSchedule {
    pub id: Uuid,
    pub name: String,
}

/// Schicht (`TimeScheduleEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleEntry {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub user_id: Option<UserId>,
    pub user_email: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Vertretung: ersetzter User; Vertretungen haben Vorrang.
    pub overridden_user_id: Option<UserId>,
}

/// Neue Schicht.
#[derive(Debug, Clone, PartialEq)]
pub struct NewScheduleEntry {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub user_id: Option<UserId>,
    pub user_email: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub overridden_user_id: Option<UserId>,
}

/// Verweis auf Team und Workspace (mandantenübergreifende Listen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TeamRef {
    pub org_id: OrgId,
    pub team_id: TeamId,
}

/// Interne Automationsspalten eines Issues.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueAutomation {
    pub auto_closed_by_parent_closing: bool,
    pub sla_rule_id: Option<Uuid>,
    pub sla_set_notified_at: Option<DateTime<Utc>>,
    pub sla_high_risk_notified_at: Option<DateTime<Utc>>,
    pub sla_breached_notified_at: Option<DateTime<Utc>>,
}

/// Meldemarke der SLA-Benachrichtigungen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlaMarker {
    Set,
    HighRisk,
    Breached,
}

/// SLA-Einstellungen eines Teams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamSla {
    pub enabled: bool,
    /// `true` = Arbeitswoche Sonntag–Donnerstag (sonst Montag–Freitag).
    pub sun_thu: bool,
}

/// Eine SLA-Regel (`sla_rule`).
#[derive(Debug, Clone, PartialEq)]
pub struct SlaRule {
    pub id: Uuid,
    pub team_id: TeamId,
    pub position: f64,
    pub name: String,
    /// `IssueFilter` als JSON (docs/05).
    pub filter: Value,
    /// `true` = Regel entfernt ein SLA.
    pub removes_sla: bool,
    /// ISO-8601-Dauer (nur bei Setz-Regeln).
    pub duration: Option<String>,
    pub sla_type: SlaType,
    /// `issueCreation` oder `ruleMatch`.
    pub start_mode: String,
}

/// Kandidat für Auto-Close mit den Angaben, die nur die Datenbank kennt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoCloseCandidate {
    pub issue_id: IssueId,
    /// Das Issue liegt in einem Cycle, der jetzt läuft (harte Ausnahme).
    pub in_active_cycle: bool,
    /// Das Issue gehört zu einem Projekt, das nicht `completed`/`canceled` ist (harte Ausnahme).
    pub in_unfinished_project: bool,
}

/// Persistenzzugriff des Bereichs `triage`.
#[async_trait]
pub trait TriageRepo: Send {
    // --- Verantwortung und Schichtpläne ---
    async fn get_responsibility(
        &mut self,
        _team: TeamId,
    ) -> AppResult<Option<TriageResponsibility>> {
        unimplemented_repo()
    }
    /// Legt die Verantwortung an oder ersetzt sie (Quelle und Aktion); `current_user_id` wird geleert.
    async fn upsert_responsibility(
        &mut self,
        _r: &NewResponsibility,
    ) -> AppResult<TriageResponsibility> {
        unimplemented_repo()
    }
    async fn delete_responsibility(&mut self, _team: TeamId) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Speichert den aktuellen Verantwortlichen (`version + 1`).
    async fn set_current_user(
        &mut self,
        _team: TeamId,
        _user: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Erhöht den Round-Robin-Zeiger um eins und liefert den bisherigen Wert.
    async fn advance_assignment_index(
        &mut self,
        _team: TeamId,
        _now: DateTime<Utc>,
    ) -> AppResult<i32> {
        unimplemented_repo()
    }
    async fn insert_schedule(
        &mut self,
        _id: Uuid,
        _name: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get_schedule(&mut self, _id: Uuid) -> AppResult<Option<TimeSchedule>> {
        unimplemented_repo()
    }
    async fn insert_schedule_entry(
        &mut self,
        _e: &NewScheduleEntry,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Schichten, die nach `ends_after` noch nicht beendet sind, nach Beginn sortiert; optional
    /// nur eines Users (auch als Vertretung).
    async fn schedule_entries(
        &mut self,
        _schedule: Uuid,
        _ends_after: DateTime<Utc>,
        _user: Option<UserId>,
        _limit: i64,
    ) -> AppResult<Vec<ScheduleEntry>> {
        unimplemented_repo()
    }
    /// Teams mit Schichtplan-Verantwortung (mandantenübergreifend).
    async fn responsibilities_with_schedule(&mut self) -> AppResult<Vec<TeamRef>> {
        unimplemented_repo()
    }

    // --- Issue-Spalten der Automationen ---
    async fn issue_automation(&mut self, _issue: IssueId) -> AppResult<IssueAutomation> {
        unimplemented_repo()
    }
    async fn set_closed_by_parent(&mut self, _issue: IssueId, _value: bool) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn set_sla_rule(&mut self, _issue: IssueId, _rule: Option<Uuid>) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn mark_sla(
        &mut self,
        _issue: IssueId,
        _marker: SlaMarker,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Setzt `archived_at` und (bei `auto`) `auto_archived_at` (`version + 1`, `updated_at`).
    async fn archive_issue(
        &mut self,
        _issue: IssueId,
        _auto: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Verschiebt die Anhänge von `from` zum kanonischen Issue `to` (Triage: Duplikat);
    /// liefert die Anzahl. Anhänge mit gleicher URL am Ziel werden nicht verschoben.
    async fn move_attachments(&mut self, _from: IssueId, _to: IssueId) -> AppResult<u64> {
        unimplemented_repo()
    }
    /// Löscht Daten ohne Fremdschlüssel, die am Issue hängen (`document_content`).
    async fn delete_issue_leftovers(&mut self, _issue: IssueId) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- Teams ---
    async fn team_sla(&mut self, _team: TeamId) -> AppResult<TeamSla> {
        unimplemented_repo()
    }
    /// Aktive Regeln eines Teams nach Position.
    async fn sla_rules(&mut self, _team: TeamId) -> AppResult<Vec<SlaRule>> {
        unimplemented_repo()
    }
    /// Teams mit `autoClosePeriod` (mandantenübergreifend, ohne Teams im Import).
    async fn teams_with_auto_close(&mut self, _now: DateTime<Utc>) -> AppResult<Vec<TeamRef>> {
        unimplemented_repo()
    }
    /// Alle aktiven Teams (mandantenübergreifend, ohne Teams im Import).
    async fn teams_with_auto_archive(&mut self, _now: DateTime<Utc>) -> AppResult<Vec<TeamRef>> {
        unimplemented_repo()
    }
    /// Workspaces mit Issues im Papierkorb (mandantenübergreifend).
    async fn orgs_with_trashed_issues(&mut self) -> AppResult<Vec<OrgId>> {
        unimplemented_repo()
    }
    /// Workspaces mit offenem SLA-Issue (mandantenübergreifend).
    async fn orgs_with_sla(&mut self) -> AppResult<Vec<OrgId>> {
        unimplemented_repo()
    }
    /// Workspaces mit abgelaufener Triage-Snooze (mandantenübergreifend).
    async fn orgs_with_due_snooze(&mut self, _now: DateTime<Utc>) -> AppResult<Vec<OrgId>> {
        unimplemented_repo()
    }

    // --- Kandidatenabfragen (je Workspace) ---
    /// Offene Issues (`triage`/`backlog`/`unstarted`) des Teams ohne Aktualisierung seit `cutoff`,
    /// aufsteigend nach ID, ab `after`.
    async fn auto_close_candidates(
        &mut self,
        _team: TeamId,
        _cutoff: DateTime<Utc>,
        _now: DateTime<Utc>,
        _after: Option<IssueId>,
        _limit: i64,
    ) -> AppResult<Vec<AutoCloseCandidate>> {
        unimplemented_repo()
    }
    /// Geschlossene Issues, die nach docs/03 §9.2 archiviert werden dürfen.
    async fn auto_archive_candidates(
        &mut self,
        _team: TeamId,
        _cutoff: DateTime<Utc>,
        _limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Issues im Papierkorb seit vor `cutoff`.
    async fn purge_candidates(
        &mut self,
        _cutoff: DateTime<Utc>,
        _limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Offene Issues mit SLA, die eine Meldung oder Neuberechnung der Schwellen brauchen.
    async fn sla_due(&mut self, _now: DateTime<Utc>, _limit: i64) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Triage-Issues, deren Snooze abgelaufen ist.
    async fn due_snoozed(&mut self, _now: DateTime<Utc>, _limit: i64) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Triage-Issues des Teams (nicht archiviert/gelöscht), nach `started_triage_at` aufsteigend;
    /// gesnoozte nur mit `include_snoozed`.
    async fn triage_issues(
        &mut self,
        _team: TeamId,
        _now: DateTime<Utc>,
        _include_snoozed: bool,
        _limit: i64,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
}
