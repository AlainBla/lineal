//! Repository-Port `CycleRepo` (WP07): Cycles, Cycle-Historie, Rollover und Job-Abfragen.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank
//! kompilieren. Die Issue-Tabelle wird hier nur für Cycle-Logik gelesen bzw. (Rollover, Auto-Zuordnung)
//! per SQL angefasst; die allgemeine Issue-Zuordnung gehört WP06.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{EstimationType, StateType};
use lineal_domain::ids::{CycleId, IssueId, OrgId, StateId, TeamId};
use lineal_domain::rules::progress::IssueProgressInput;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Cycle (docs/04 §1.8). `is_*`-Flags werden aus `now` berechnet, nicht gespeichert.
#[derive(Debug, Clone, PartialEq)]
pub struct Cycle {
    pub id: CycleId,
    pub org_id: OrgId,
    pub team_id: TeamId,
    pub number: i32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub inherited_from_id: Option<CycleId>,
    /// Intern: Phase 2 des Wechsels (Start und Rollover) wurde verarbeitet.
    pub started_processed_at: Option<DateTime<Utc>>,
    /// Beim Abschluss eingefrorener Fortschritt (0..1); davor `None` (wird berechnet).
    pub progress: Option<f64>,
    /// Beim Abschluss offene Issues (eingefroren).
    pub uncompleted_issue_ids: Option<Vec<Uuid>>,
    pub scope_history: Vec<f64>,
    pub completed_scope_history: Vec<f64>,
    pub in_progress_scope_history: Vec<f64>,
    pub issue_count_history: Vec<f64>,
    pub completed_issue_count_history: Vec<f64>,
    pub auto_archived_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Cycle {
    /// Läuft gerade (`startsAt <= now < endsAt`, nicht abgeschlossen).
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.completed_at.is_none() && self.starts_at <= now && now < self.ends_at
    }
    /// Abgeschlossen (`completedAt` gesetzt).
    pub fn is_past(&self) -> bool {
        self.completed_at.is_some()
    }
    /// Beginnt in der Zukunft.
    pub fn is_future(&self, now: DateTime<Utc>) -> bool {
        self.starts_at > now
    }
}

/// Neuer Cycle (Nummer kommt vom Zähler des Teams).
#[derive(Debug, Clone, PartialEq)]
pub struct NewCycle {
    pub id: CycleId,
    pub team_id: TeamId,
    pub number: i32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub inherited_from_id: Option<CycleId>,
    pub now: DateTime<Utc>,
}

/// Ein Issue eines Cycles/Projekts/Meilensteins mit den Daten für Scope und Fortschritt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IssueScopeRow {
    pub issue_id: IssueId,
    pub team_id: TeamId,
    pub state_type: StateType,
    pub estimate: Option<i32>,
    /// Einstellungen des **eigenen** Teams des Issues (docs/04 §2.9).
    pub estimation_type: EstimationType,
    pub default_estimate: i32,
}

impl IssueScopeRow {
    /// Eingabe der Fortschrittsformel (`rules::progress`).
    pub fn progress_input(&self) -> IssueProgressInput {
        IssueProgressInput {
            state_type: self.state_type,
            estimate: self.estimate.map(f64::from),
            estimation_type: self.estimation_type,
            default_estimate: f64::from(self.default_estimate),
        }
    }
}

/// Tageswerte der fünf Historien-Arrays (docs/04 §1.9).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HistoryPoint {
    pub scope: f64,
    pub completed_scope: f64,
    pub in_progress_scope: f64,
    pub issue_count: f64,
    pub completed_issue_count: f64,
}

/// Team, für das der Wechsel-Job laufen muss (mandantenübergreifend, nur mit `begin_system`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleTeamRef {
    pub org_id: OrgId,
    pub team_id: TeamId,
}

/// Persistenzzugriff des Bereichs `cycle`.
#[async_trait]
pub trait CycleRepo: Send {
    async fn insert(&mut self, _cycle: &NewCycle) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: CycleId) -> AppResult<Option<Cycle>> {
        unimplemented_repo()
    }
    /// Alle Cycles eines Teams, aufsteigend nach `starts_at`.
    async fn list_for_team(
        &mut self,
        _team: TeamId,
        _include_archived: bool,
    ) -> AppResult<Vec<Cycle>> {
        unimplemented_repo()
    }
    /// Cycles, die einem Cycle des Parent-Teams folgen (`inherited_from_id = parent`).
    async fn list_inheriting(&mut self, _parent: CycleId) -> AppResult<Vec<Cycle>> {
        unimplemented_repo()
    }
    /// Name und Beschreibung ändern (`Some(None)` leert).
    async fn update_meta(
        &mut self,
        _id: CycleId,
        _name: Option<Option<String>>,
        _description: Option<Option<String>>,
        _now: DateTime<Utc>,
    ) -> AppResult<Cycle> {
        unimplemented_repo()
    }
    /// Setzt Beginn und Ende. Verletzt das Ergebnis die Überlappungsfreiheit → `conflict`.
    async fn update_times(
        &mut self,
        _id: CycleId,
        _starts_at: DateTime<Utc>,
        _ends_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<Cycle> {
        unimplemented_repo()
    }
    /// Setzt `inherited_from_id` (z. B. `None`, wenn ein Sub-Team eigenständig wird).
    async fn set_inherited_from(
        &mut self,
        _id: CycleId,
        _parent: Option<CycleId>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Löscht den Cycle endgültig; die Issues verlieren die Zuordnung (FK `on delete set null`).
    async fn delete(&mut self, _id: CycleId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Markiert den Cycle als archiviert.
    async fn archive(&mut self, _id: CycleId, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }

    /// Issues des Cycles (ohne Papierkorb) mit Daten für Scope und Fortschritt.
    async fn issue_rows(&mut self, _cycle: CycleId) -> AppResult<Vec<IssueScopeRow>> {
        unimplemented_repo()
    }
    /// Offene Issues (`triage|backlog|unstarted|started`) des Cycles.
    async fn open_issue_ids(&mut self, _cycle: CycleId) -> AppResult<Vec<Uuid>> {
        unimplemented_repo()
    }
    /// Zustand von `ids`, soweit sie noch im Cycle `cycle` liegen (Rollover).
    async fn issue_states_in_cycle(
        &mut self,
        _cycle: CycleId,
        _ids: &[Uuid],
    ) -> AppResult<Vec<(IssueId, StateType)>> {
        unimplemented_repo()
    }
    /// Alle Issues des Cycles (auch abgeschlossene), z. B. vor dem Löschen.
    async fn all_issue_ids(&mut self, _cycle: CycleId) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Ordnet ein Issue einem Cycle zu (oder löst es), setzt `added_to_cycle_at`, erhöht
    /// `version` und schreibt `issue_history` (Quelle `system`, `changes.cycleId`). Liefert das Team
    /// des Issues; das `IssueUpdated`-Event (`changed = ["cycleId"]`) schreibt der Use-Case.
    async fn set_issue_cycle(
        &mut self,
        _issue: IssueId,
        _from: Option<CycleId>,
        _to: Option<CycleId>,
        _now: DateTime<Utc>,
    ) -> AppResult<TeamId> {
        unimplemented_repo()
    }
    /// Schreibt die Tageswerte an Index `idx` der fünf Historien-Arrays (Lücken werden mit dem
    /// letzten Wert aufgefüllt). Abgeschlossene Cycles bleiben unverändert (Historie eingefroren).
    async fn write_history_point(
        &mut self,
        _id: CycleId,
        _idx: usize,
        _point: HistoryPoint,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Schließt den Cycle ab: `completed_at`, `ends_at` (bei vorzeitigem Abschluss gekürzt),
    /// eingefrorene Liste und eingefrorener Fortschritt.
    async fn close(
        &mut self,
        _id: CycleId,
        _completed_at: DateTime<Utc>,
        _ends_at: DateTime<Utc>,
        _uncompleted: &[Uuid],
        _progress: f64,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Vermerkt, dass Start und Rollover des Cycles verarbeitet wurden.
    async fn mark_started(&mut self, _id: CycleId, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }

    /// Teams mit aktivierten Cycles (nicht gelöscht/stillgelegt, kein laufender Import) über
    /// **alle** Workspaces; nur in einer `begin_system`-Transaktion zu verwenden.
    async fn teams_with_cycles(&mut self, _now: DateTime<Utc>) -> AppResult<Vec<CycleTeamRef>> {
        unimplemented_repo()
    }
    /// Sperrt die Team-Zeile (`FOR UPDATE`) als „Team-Sperre“ des Wechsel-Jobs.
    async fn lock_team(&mut self, _team: TeamId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Teams, deren `inherits_cycles` auf `false` steht (Sub-Team ist eigenständig).
    async fn teams_not_inheriting(&mut self) -> AppResult<Vec<TeamId>> {
        unimplemented_repo()
    }
    /// Fingerprint, zu dem die künftigen Cycles des Teams zuletzt berechnet wurden.
    async fn team_schedule_fp(&mut self, _team: TeamId) -> AppResult<Option<String>> {
        unimplemented_repo()
    }
    async fn set_team_schedule_fp(&mut self, _team: TeamId, _fp: Option<&str>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Teams **ohne** aktivierte Cycles, die noch offene (nicht abgeschlossene, nicht archivierte)
    /// Cycles besitzen (Deaktivierung wurde an anderer Stelle gesetzt); nur `begin_system`.
    async fn teams_disabled_with_open_cycles(&mut self) -> AppResult<Vec<CycleTeamRef>> {
        unimplemented_repo()
    }
    async fn set_team_inherits(
        &mut self,
        _team: TeamId,
        _inherits: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// „Active issues“ aktivieren, `addToCycle`: ordnet alle Issues in `unstarted|started` ohne Cycle
    /// dem Cycle zu; liefert die geänderten Issues.
    async fn assign_active_issues_without_cycle(
        &mut self,
        _team: TeamId,
        _cycle: CycleId,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// „Active issues“ aktivieren, `moveToBacklog`: setzt alle Issues in `unstarted|started` ohne
    /// Cycle in den Backlog-Status `backlog_state`; liefert die geänderten Issues.
    async fn move_active_issues_without_cycle_to_backlog(
        &mut self,
        _team: TeamId,
        _backlog_state: StateId,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
}
