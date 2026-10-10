//! Repository-Port `ProjectRepo` (WP07): Projekte, Projektstatus, Meilensteine, Status-Updates.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank
//! kompilieren. Die Issue-Tabelle wird nur gelesen (Fortschritt, Scope) bzw. bei Teamentfernung und
//! Meilenstein-Löschung per SQL von der Projektzuordnung gelöst; die allgemeine Zuordnung gehört WP06.

use super::cycle::IssueScopeRow;
use super::user::{Page, PageRequest};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::{DateResolution, Health, ProjectStatusType};
use lineal_domain::ids::{
    InitiativeId, IssueId, MilestoneId, OrgId, ProjectId, ProjectUpdateId, TeamId, UserId,
};
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Projektstatus (docs/04 §2.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectStatus {
    pub id: Uuid,
    pub name: String,
    pub status_type: ProjectStatusType,
    pub color: String,
    pub description: Option<String>,
    pub position: f64,
    pub indefinite: bool,
    /// `None` = Workspace-Status.
    pub team_id: Option<TeamId>,
    pub inherited_from_id: Option<Uuid>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Ein Projekt (docs/04 §2.1). Teams, Mitglieder und Labels liegen in eigenen Abfragen.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    pub org_id: OrgId,
    pub name: String,
    pub lead_team_id: TeamId,
    pub number: i32,
    pub slug_id: String,
    /// Kurzbeschreibung (MCP `summary`), höchstens 255 Zeichen.
    pub description: Option<String>,
    /// Markdown-Übersicht (MCP `description`).
    pub content: Option<String>,
    pub color: String,
    pub icon: Option<String>,
    pub status_id: Uuid,
    pub priority: i32,
    pub sort_order: f64,
    pub priority_sort_order: f64,
    pub lead_id: Option<UserId>,
    pub creator_id: Option<UserId>,
    pub start_date: Option<NaiveDate>,
    pub start_date_resolution: Option<DateResolution>,
    pub target_date: Option<NaiveDate>,
    pub target_date_resolution: Option<DateResolution>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub health: Option<Health>,
    pub health_updated_at: Option<DateTime<Utc>>,
    pub last_update_id: Option<ProjectUpdateId>,
    pub update_reminder_mode: String,
    pub update_reminder_frequency_in_weeks: Option<i32>,
    pub update_reminders_day: Option<String>,
    pub update_reminders_hour: Option<i32>,
    pub update_reminders_paused_until_at: Option<DateTime<Utc>>,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub auto_archived_at: Option<DateTime<Utc>>,
    pub scope_history: Vec<f64>,
    pub completed_scope_history: Vec<f64>,
    pub in_progress_scope_history: Vec<f64>,
    pub issue_count_history: Vec<f64>,
    pub completed_issue_count_history: Vec<f64>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    /// Lesbare Kennung `P-<KEY>-<n>` zu einem Team-Key (Key des Lead-Teams).
    pub fn identifier(&self, lead_team_key: &str) -> String {
        format!("P-{}-{}", lead_team_key, self.number)
    }
}

/// Neues Projekt (Nummer vom Zähler des Lead-Teams).
#[derive(Debug, Clone, PartialEq)]
pub struct NewProject {
    pub id: ProjectId,
    pub name: String,
    pub lead_team_id: TeamId,
    pub number: i32,
    pub description: Option<String>,
    pub content: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status_id: Uuid,
    pub priority: i32,
    pub sort_order: f64,
    pub lead_id: Option<UserId>,
    pub creator_id: Option<UserId>,
    pub start_date: Option<NaiveDate>,
    pub start_date_resolution: Option<DateResolution>,
    pub target_date: Option<NaiveDate>,
    pub target_date_resolution: Option<DateResolution>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

/// Änderung eines Projekts. `None` = unverändert; `Some(None)` leert nullbare Felder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub lead_team_id: Option<TeamId>,
    pub number: Option<i32>,
    pub description: Option<Option<String>>,
    pub content: Option<Option<String>>,
    pub color: Option<String>,
    pub icon: Option<Option<String>>,
    pub status_id: Option<Uuid>,
    pub priority: Option<i32>,
    pub sort_order: Option<f64>,
    pub lead_id: Option<Option<UserId>>,
    pub start_date: Option<Option<NaiveDate>>,
    pub start_date_resolution: Option<Option<DateResolution>>,
    pub target_date: Option<Option<NaiveDate>>,
    pub target_date_resolution: Option<Option<DateResolution>>,
    pub started_at: Option<Option<DateTime<Utc>>>,
    pub completed_at: Option<Option<DateTime<Utc>>>,
    pub canceled_at: Option<Option<DateTime<Utc>>>,
    pub health: Option<Option<Health>>,
    pub health_updated_at: Option<Option<DateTime<Utc>>>,
    pub last_update_id: Option<Option<ProjectUpdateId>>,
    pub update_reminder_mode: Option<String>,
    pub update_reminder_frequency_in_weeks: Option<Option<i32>>,
    pub update_reminders_day: Option<Option<String>>,
    pub update_reminders_hour: Option<Option<i32>>,
    pub update_reminders_paused_until_at: Option<Option<DateTime<Utc>>>,
    pub trashed: Option<bool>,
    pub archived_at: Option<Option<DateTime<Utc>>>,
}

/// Filter von `list_projects` (alle Bedingungen mit UND, docs/04 §7.2). Referenzen sind aufgelöst.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectFilter {
    /// Nur Projekte, an denen mindestens eines dieser Teams beteiligt ist (Sichtbarkeit).
    pub visible_teams: Vec<TeamId>,
    /// Nur Projekte, an denen eines dieser Teams beteiligt ist.
    pub teams: Option<Vec<TeamId>>,
    /// Nur Projekte dieser Initiativen (bereits um Sub-Initiativen erweitert).
    pub initiatives: Option<Vec<InitiativeId>>,
    /// Lead oder Mitglied.
    pub member: Option<UserId>,
    pub status_ids: Option<Vec<Uuid>>,
    pub label_ids: Option<Vec<Uuid>>,
    /// Teilstring in Name oder Kurzbeschreibung.
    pub query: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
}

/// Ein Projekt-Label (Lesezugriff; Verwaltung gehört WP06).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectLabelRow {
    pub id: Uuid,
    pub name: String,
    pub team_id: Option<TeamId>,
    pub parent_id: Option<Uuid>,
    pub is_group: bool,
    pub retired: bool,
}

/// Ein externer Link (Ressource).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLink {
    pub id: Uuid,
    pub url: String,
    pub label: String,
    pub sort_order: f64,
}

/// Ein Meilenstein (docs/04 §3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Milestone {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<NaiveDate>,
    pub sort_order: f64,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMilestone {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<NaiveDate>,
    pub sort_order: f64,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MilestonePatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub target_date: Option<Option<NaiveDate>>,
    pub sort_order: Option<f64>,
}

/// Ein Status-Update eines Projekts (docs/04 §5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectUpdate {
    pub id: ProjectUpdateId,
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub body: String,
    pub health: Health,
    pub slug_id: String,
    pub edited_at: Option<DateTime<Utc>>,
    /// Interner Snapshot des Projekts zum Zeitpunkt des Updates (Basis des nächsten Diffs).
    pub snapshot: serde_json::Value,
    pub diff: Option<serde_json::Value>,
    pub diff_markdown: Option<String>,
    pub is_diff_hidden: bool,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewProjectUpdate {
    pub id: ProjectUpdateId,
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub body: String,
    pub health: Health,
    pub snapshot: serde_json::Value,
    pub diff: Option<serde_json::Value>,
    pub diff_markdown: Option<String>,
    pub is_diff_hidden: bool,
    pub now: DateTime<Utc>,
}

/// Filter von `get_status_updates` (Projekt).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectUpdateFilter {
    /// Sichtbarkeit: nur Updates von Projekten mit einem dieser Teams.
    pub visible_teams: Vec<TeamId>,
    pub project: Option<ProjectId>,
    pub user: Option<UserId>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
}

/// Kandidat der Erinnerungs-Jobs (Projekt in `started` ohne `indefinite`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderTarget {
    pub target_id: Uuid,
    pub name: String,
    /// Lead (Projekt) bzw. Owner (Initiative); ohne Empfänger gibt es keine Erinnerung.
    pub recipient_id: Option<UserId>,
    pub recipient_timezone: Option<String>,
    pub mode: String,
    pub frequency_in_weeks: Option<i32>,
    pub day: Option<String>,
    pub hour: Option<i32>,
    pub paused_until: Option<DateTime<Utc>>,
    pub last_update_at: Option<DateTime<Utc>>,
    /// Beginn der Zählung: `startedAt` bzw. `createdAt`.
    pub since: DateTime<Utc>,
}

/// Workspace-Einstellungen der Erinnerungen (docs/04 §5.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderSettings {
    pub org_id: OrgId,
    pub project_frequency_in_weeks: Option<i32>,
    pub project_day: String,
    pub project_hour: i32,
    pub initiative_frequency_in_weeks: Option<i32>,
    pub initiative_day: String,
    pub initiative_hour: i32,
}

/// Persistenzzugriff des Bereichs `project`.
#[async_trait]
pub trait ProjectRepo: Send {
    // --- Projektstatus -------------------------------------------------------------------
    /// Alle Projektstatus des Workspace (auch archivierte), nach Kategorie und `position`.
    async fn list_statuses(&mut self) -> AppResult<Vec<ProjectStatus>> {
        unimplemented_repo()
    }

    // --- Projekte ------------------------------------------------------------------------
    async fn insert(&mut self, _project: &NewProject) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: ProjectId) -> AppResult<Option<Project>> {
        unimplemented_repo()
    }
    /// Wendet den Patch an (`version + 1`, `updated_at = now`) und liefert den neuen Stand.
    async fn update(
        &mut self,
        _id: ProjectId,
        _patch: &ProjectPatch,
        _now: DateTime<Utc>,
    ) -> AppResult<Project> {
        unimplemented_repo()
    }
    /// Projekt über Key des Lead-Teams (aktuell) und Nummer (`P-ENG-7`).
    async fn find_by_key_and_number(
        &mut self,
        _team_key: &str,
        _number: i32,
    ) -> AppResult<Option<Project>> {
        unimplemented_repo()
    }
    /// Projekt über eine frühere Kennung (`previousIdentifiers`).
    async fn find_by_alias(&mut self, _identifier: &str) -> AppResult<Option<Project>> {
        unimplemented_repo()
    }
    async fn find_by_slug_id(&mut self, _slug_id: &str) -> AppResult<Option<Project>> {
        unimplemented_repo()
    }
    /// Projekte mit genau diesem Namen (case-insensitiv), nicht im Papierkorb.
    async fn find_by_name(&mut self, _name: &str) -> AppResult<Vec<Project>> {
        unimplemented_repo()
    }
    /// Liste nach `docs/00 §5` (absteigend nach `order_by`, Tiebreaker `id`).
    async fn list(
        &mut self,
        _filter: &ProjectFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<Project>> {
        unimplemented_repo()
    }
    /// Frühere Kennungen (`previousIdentifiers`) des Projekts.
    async fn previous_identifiers(&mut self, _id: ProjectId) -> AppResult<Vec<String>> {
        unimplemented_repo()
    }
    async fn add_alias(
        &mut self,
        _id: ProjectId,
        _identifier: &str,
        _team_key: &str,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Projekte, die im Papierkorb liegen sollen bzw. Teamlöschung: Projekte mit einem Team.
    async fn ids_for_team(&mut self, _team: TeamId) -> AppResult<Vec<ProjectId>> {
        unimplemented_repo()
    }

    // --- Teams, Mitglieder, Labels, Links ------------------------------------------------
    /// Beteiligte Teams (nach `sort_order`).
    async fn list_teams(&mut self, _id: ProjectId) -> AppResult<Vec<TeamId>> {
        unimplemented_repo()
    }
    async fn add_team(
        &mut self,
        _id: ProjectId,
        _team: TeamId,
        _sort_order: f64,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn remove_team(&mut self, _id: ProjectId, _team: TeamId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Teams mehrerer Projekte in einem Zug (Listen): `(project, team)` nach `sort_order`.
    async fn teams_of_projects(
        &mut self,
        _ids: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, TeamId)>> {
        unimplemented_repo()
    }
    async fn list_members(&mut self, _id: ProjectId) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    async fn add_member(
        &mut self,
        _id: ProjectId,
        _user: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn remove_member(&mut self, _id: ProjectId, _user: UserId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Labels des Projekts.
    async fn labels_of(&mut self, _id: ProjectId) -> AppResult<Vec<ProjectLabelRow>> {
        unimplemented_repo()
    }
    /// Projekt-Label per UUID.
    async fn get_label(&mut self, _id: Uuid) -> AppResult<Option<ProjectLabelRow>> {
        unimplemented_repo()
    }
    /// Projekt-Labels (nicht archiviert) mit diesem Namen, case-insensitiv.
    async fn find_labels_by_name(&mut self, _name: &str) -> AppResult<Vec<ProjectLabelRow>> {
        unimplemented_repo()
    }
    /// Ersetzt die Label-Zuordnung des Projekts. `group_id` wird aus `parent_id` des Labels gesetzt.
    async fn set_labels(
        &mut self,
        _id: ProjectId,
        _labels: &[ProjectLabelRow],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn list_links(&mut self, _id: ProjectId) -> AppResult<Vec<ExternalLink>> {
        unimplemented_repo()
    }
    /// Hängt einen externen Link an; ein bereits vorhandener Link (gleiche URL) bleibt unverändert.
    async fn add_link(
        &mut self,
        _id: ProjectId,
        _url: &str,
        _label: &str,
        _creator: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Anzahl Dokumente des Projekts (`resourceCount`).
    async fn document_count(&mut self, _id: ProjectId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Dokumente des Projekts (`id`, Titel, Slug) für `includeResources`.
    async fn list_documents(&mut self, _id: ProjectId) -> AppResult<Vec<(Uuid, String, String)>> {
        unimplemented_repo()
    }
    /// Lead als Abonnent des Projekts eintragen (idempotent).
    async fn subscribe(
        &mut self,
        _id: ProjectId,
        _user: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- Issues (nur lesen bzw. Zuordnung lösen) ------------------------------------------
    /// Alle Issues des Projekts (archivierte zählen, getrashte nicht) für Scope und Fortschritt.
    async fn issue_rows(&mut self, _id: ProjectId) -> AppResult<Vec<IssueScopeRow>> {
        unimplemented_repo()
    }
    /// Fortschrittszeilen mehrerer Projekte in einem Zug.
    async fn issue_rows_of_projects(
        &mut self,
        _ids: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, IssueScopeRow)>> {
        unimplemented_repo()
    }
    /// Anzahl offener Issues (`triage|backlog|unstarted|started`).
    async fn open_issue_count(&mut self, _id: ProjectId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Issues eines Teams im Projekt (Kennung, Id), höchstens `limit`.
    async fn issues_of_team(
        &mut self,
        _id: ProjectId,
        _team: TeamId,
        _limit: i64,
    ) -> AppResult<Vec<(IssueId, String)>> {
        unimplemented_repo()
    }
    /// Löst alle Issues des Teams vom Projekt und von dessen Meilensteinen (`project_id`,
    /// `project_milestone_id` = null, `version + 1`, `issue_history`-Eintrag mit Quelle `user` und
    /// Akteur). Liefert die betroffenen Issues.
    async fn detach_team_issues(
        &mut self,
        _id: ProjectId,
        _team: TeamId,
        _actor: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }

    // --- Meilensteine ---------------------------------------------------------------------
    async fn list_milestones(&mut self, _id: ProjectId) -> AppResult<Vec<Milestone>> {
        unimplemented_repo()
    }
    async fn get_milestone(&mut self, _id: MilestoneId) -> AppResult<Option<Milestone>> {
        unimplemented_repo()
    }
    /// Meilenstein mit diesem Namen im Projekt (case-insensitiv).
    async fn find_milestone_by_name(
        &mut self,
        _project: ProjectId,
        _name: &str,
    ) -> AppResult<Option<Milestone>> {
        unimplemented_repo()
    }
    async fn insert_milestone(&mut self, _m: &NewMilestone) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn update_milestone(
        &mut self,
        _id: MilestoneId,
        _patch: &MilestonePatch,
        _now: DateTime<Utc>,
    ) -> AppResult<Milestone> {
        unimplemented_repo()
    }
    /// Größtes `sort_order` im Projekt (0, wenn es keine Meilensteine gibt).
    async fn max_milestone_sort_order(&mut self, _project: ProjectId) -> AppResult<f64> {
        unimplemented_repo()
    }
    /// Archiviert den Meilenstein und löst seine Issues (History-Eintrag je Issue); liefert die
    /// Issues mit ihrem Team.
    async fn archive_milestone(
        &mut self,
        _id: MilestoneId,
        _actor: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<(IssueId, TeamId)>> {
        unimplemented_repo()
    }
    /// Fortschrittszeilen der Issues je Meilenstein des Projekts.
    async fn milestone_issue_rows(
        &mut self,
        _project: ProjectId,
    ) -> AppResult<Vec<(MilestoneId, IssueScopeRow)>> {
        unimplemented_repo()
    }

    // --- Status-Updates -------------------------------------------------------------------
    async fn insert_update(&mut self, _u: &NewProjectUpdate) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get_update(&mut self, _id: ProjectUpdateId) -> AppResult<Option<ProjectUpdate>> {
        unimplemented_repo()
    }
    /// Das jüngste nicht archivierte Update (optional ohne `except`).
    async fn latest_update(
        &mut self,
        _project: ProjectId,
        _except: Option<ProjectUpdateId>,
    ) -> AppResult<Option<ProjectUpdate>> {
        unimplemented_repo()
    }
    async fn list_updates(
        &mut self,
        _filter: &ProjectUpdateFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<ProjectUpdate>> {
        unimplemented_repo()
    }
    /// Bearbeitet das Update (`edited_at = now`); `None` = unverändert.
    async fn edit_update(
        &mut self,
        _id: ProjectUpdateId,
        _body: Option<&str>,
        _health: Option<Health>,
        _now: DateTime<Utc>,
    ) -> AppResult<ProjectUpdate> {
        unimplemented_repo()
    }
    async fn archive_update(&mut self, _id: ProjectUpdateId, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Anzahl Kommentare im Thread des Updates.
    async fn update_comment_count(&mut self, _id: ProjectUpdateId) -> AppResult<i64> {
        unimplemented_repo()
    }

    // --- Erinnerungen und Snapshots (Jobs) ------------------------------------------------
    /// Erinnerungseinstellungen des Workspace dieser Transaktion.
    async fn reminder_settings(&mut self) -> AppResult<Option<ReminderSettings>> {
        unimplemented_repo()
    }
    /// Projekte im Status `started` (nicht `indefinite`), nicht archiviert/getrasht.
    async fn reminder_targets(&mut self) -> AppResult<Vec<ReminderTarget>> {
        unimplemented_repo()
    }
    /// Alle Workspaces mit Projekten oder Initiativen (nur `begin_system`).
    async fn orgs_with_planning(&mut self) -> AppResult<Vec<OrgId>> {
        unimplemented_repo()
    }
    /// Ob für `(target, due_at, stage)` bereits erinnert wurde.
    async fn reminder_sent(
        &mut self,
        _target_type: &str,
        _target: Uuid,
        _due_at: DateTime<Utc>,
        _stage: i32,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Vermerkt die Erinnerung und legt die Benachrichtigung `projectUpdatePrompt` bzw.
    /// `initiativeUpdatePrompt` (Kategorie `reminders`) für den Empfänger an.
    async fn record_reminder(
        &mut self,
        _target_type: &str,
        _target: Uuid,
        _recipient: UserId,
        _due_at: DateTime<Utc>,
        _stage: i32,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Schreibt den Wochenwert der fünf Historien-Arrays eines Projekts (Index = Wochen seit Anlage).
    async fn write_history_point(
        &mut self,
        _id: ProjectId,
        _idx: usize,
        _point: super::cycle::HistoryPoint,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Aktive Projekte (nicht abgeschlossen/abgebrochen, nicht archiviert/getrasht) für Wochen-Snapshots.
    async fn active_project_ids(&mut self) -> AppResult<Vec<ProjectId>> {
        unimplemented_repo()
    }
}
