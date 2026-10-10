//! Repository-Port `InitiativeRepo` (WP07): Initiativen, Hierarchie (DAG), Projektzuordnung, Updates.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank
//! kompilieren.

use super::project::{ReminderSettings, ReminderTarget};
use super::user::{Page, PageRequest};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::{DateResolution, Health, InitiativeStatus};
use lineal_domain::ids::{InitiativeId, InitiativeUpdateId, OrgId, ProjectId, TeamId, UserId};
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Eine Initiative (docs/04 §4.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Initiative {
    pub id: InitiativeId,
    pub org_id: OrgId,
    pub number: i32,
    pub name: String,
    pub slug_id: String,
    /// Kurzbeschreibung (MCP `summary`), höchstens 255 Zeichen.
    pub description: Option<String>,
    /// Markdown-Beschreibung (MCP `description`).
    pub content: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status: InitiativeStatus,
    pub owner_id: Option<UserId>,
    pub creator_id: Option<UserId>,
    pub lead_team_id: Option<TeamId>,
    pub priority: i32,
    pub priority_sort_order: f64,
    pub sort_order: f64,
    pub target_date: Option<NaiveDate>,
    pub target_date_resolution: Option<DateResolution>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub health: Option<Health>,
    pub health_updated_at: Option<DateTime<Utc>>,
    pub last_update_id: Option<InitiativeUpdateId>,
    pub update_reminder_mode: String,
    pub update_reminder_frequency_in_weeks: Option<i32>,
    pub update_reminders_day: Option<String>,
    pub update_reminders_hour: Option<i32>,
    pub include_sub_initiative_projects: bool,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Initiative {
    /// Lesbare Kennung `I-<n>`.
    pub fn identifier(&self) -> String {
        format!("I-{}", self.number)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewInitiative {
    pub id: InitiativeId,
    pub number: i32,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub status: InitiativeStatus,
    pub owner_id: Option<UserId>,
    pub creator_id: Option<UserId>,
    pub lead_team_id: Option<TeamId>,
    pub priority: i32,
    pub sort_order: f64,
    pub target_date: Option<NaiveDate>,
    pub target_date_resolution: Option<DateResolution>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

/// Änderung einer Initiative. `None` = unverändert; `Some(None)` leert nullbare Felder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitiativePatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub content: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub status: Option<InitiativeStatus>,
    pub owner_id: Option<Option<UserId>>,
    pub lead_team_id: Option<Option<TeamId>>,
    pub priority: Option<i32>,
    pub sort_order: Option<f64>,
    pub target_date: Option<Option<NaiveDate>>,
    pub target_date_resolution: Option<Option<DateResolution>>,
    pub started_at: Option<Option<DateTime<Utc>>>,
    pub completed_at: Option<Option<DateTime<Utc>>>,
    pub health: Option<Option<Health>>,
    pub health_updated_at: Option<Option<DateTime<Utc>>>,
    pub last_update_id: Option<Option<InitiativeUpdateId>>,
    pub update_reminder_mode: Option<String>,
    pub update_reminder_frequency_in_weeks: Option<Option<i32>>,
    pub update_reminders_day: Option<Option<String>>,
    pub update_reminders_hour: Option<Option<i32>>,
    pub include_sub_initiative_projects: Option<bool>,
    pub trashed: Option<bool>,
    pub archived_at: Option<Option<DateTime<Utc>>>,
}

/// Filter von `list_initiatives` (UND-verknüpft, docs/04 §7.3). Referenzen sind aufgelöst.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitiativeFilter {
    pub status: Option<InitiativeStatus>,
    pub owner: Option<UserId>,
    pub lead_team: Option<TeamId>,
    /// Direkte Kinder dieser Initiative.
    pub parent: Option<InitiativeId>,
    pub label_ids: Option<Vec<Uuid>>,
    pub query: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    /// Sichtbarkeit: Initiativen ohne Lead-Team oder mit Lead-Team aus dieser Menge.
    pub visible_teams: Vec<TeamId>,
    /// Filterbaum einer Custom View (WP11); setzt `eval` voraus, immer UND mit der Sichtbarkeit.
    pub ast: Option<Box<lineal_domain::rules::filter::InitiativeFilter>>,
    pub eval: Option<super::view::FilterEval>,
}

/// Ein Initiative-Label (Lesezugriff; Verwaltung gehört WP06).
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeLabelRow {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub is_group: bool,
    pub retired: bool,
}

/// Ein Status-Update einer Initiative (docs/04 §5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeUpdate {
    pub id: InitiativeUpdateId,
    pub initiative_id: InitiativeId,
    pub user_id: UserId,
    pub body: String,
    pub health: Health,
    pub slug_id: String,
    pub edited_at: Option<DateTime<Utc>>,
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
pub struct NewInitiativeUpdate {
    pub id: InitiativeUpdateId,
    pub initiative_id: InitiativeId,
    pub user_id: UserId,
    pub body: String,
    pub health: Health,
    pub snapshot: serde_json::Value,
    pub diff: Option<serde_json::Value>,
    pub diff_markdown: Option<String>,
    pub is_diff_hidden: bool,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitiativeUpdateFilter {
    /// Sichtbarkeit (wie [`InitiativeFilter::visible_teams`]).
    pub visible_teams: Vec<TeamId>,
    pub initiative: Option<InitiativeId>,
    pub user: Option<UserId>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
}

/// Persistenzzugriff des Bereichs `initiative`.
#[async_trait]
pub trait InitiativeRepo: Send {
    /// Nächste Initiative-Nummer (`organization.last_initiative_number`), atomar.
    async fn next_number(&mut self) -> AppResult<i32> {
        unimplemented_repo()
    }
    /// Serialisiert Änderungen am Initiative-Graphen (sperrt die Workspace-Zeile `FOR UPDATE`),
    /// damit zwei parallele Operationen keinen Zyklus erzeugen (docs/04 §4.3).
    async fn lock_hierarchy(&mut self) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn insert(&mut self, _i: &NewInitiative) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: InitiativeId) -> AppResult<Option<Initiative>> {
        unimplemented_repo()
    }
    async fn update(
        &mut self,
        _id: InitiativeId,
        _patch: &InitiativePatch,
        _now: DateTime<Utc>,
    ) -> AppResult<Initiative> {
        unimplemented_repo()
    }
    async fn find_by_number(&mut self, _number: i32) -> AppResult<Option<Initiative>> {
        unimplemented_repo()
    }
    async fn find_by_slug_id(&mut self, _slug_id: &str) -> AppResult<Option<Initiative>> {
        unimplemented_repo()
    }
    /// Initiativen mit genau diesem Namen (case-insensitiv), nicht im Papierkorb.
    async fn find_by_name(&mut self, _name: &str) -> AppResult<Vec<Initiative>> {
        unimplemented_repo()
    }
    async fn list(
        &mut self,
        _filter: &InitiativeFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<Initiative>> {
        unimplemented_repo()
    }
    /// Alle Initiativen (nicht im Papierkorb), z. B. für Graph-Algorithmen und Sichtbarkeit.
    async fn list_all(&mut self) -> AppResult<Vec<Initiative>> {
        unimplemented_repo()
    }

    // --- Hierarchie ------------------------------------------------------------------------
    /// Alle Kanten `(parent, child)` des Workspace.
    async fn list_edges(&mut self) -> AppResult<Vec<(InitiativeId, InitiativeId)>> {
        unimplemented_repo()
    }
    /// Legt eine Kante an; eine vorhandene Kante bleibt unverändert (No-op).
    async fn insert_edge(
        &mut self,
        _parent: InitiativeId,
        _child: InitiativeId,
        _sort_order: f64,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Entfernt eine Kante; `true`, wenn sie existierte.
    async fn delete_edge(
        &mut self,
        _parent: InitiativeId,
        _child: InitiativeId,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Größtes `sort_order` der Kinder von `parent` (0 ohne Kinder).
    async fn max_child_sort_order(&mut self, _parent: InitiativeId) -> AppResult<f64> {
        unimplemented_repo()
    }

    // --- Projekte ---------------------------------------------------------------------------
    /// Direkt zugeordnete Projekte nach `sort_order`.
    async fn project_ids(&mut self, _id: InitiativeId) -> AppResult<Vec<ProjectId>> {
        unimplemented_repo()
    }
    /// Initiativen, denen ein Projekt direkt zugeordnet ist.
    async fn initiative_ids_of_project(
        &mut self,
        _project: ProjectId,
    ) -> AppResult<Vec<InitiativeId>> {
        unimplemented_repo()
    }
    /// Zuordnung Initiative ↔ Projekt; `true`, wenn neu.
    async fn link_project(
        &mut self,
        _initiative: InitiativeId,
        _project: ProjectId,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    async fn unlink_project(
        &mut self,
        _initiative: InitiativeId,
        _project: ProjectId,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Zuordnungen mehrerer Projekte in einem Zug: `(project, initiative)`.
    async fn initiatives_of_projects(
        &mut self,
        _projects: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, InitiativeId)>> {
        unimplemented_repo()
    }

    // --- Labels (lesen) -----------------------------------------------------------------------
    async fn labels_of(&mut self, _id: InitiativeId) -> AppResult<Vec<InitiativeLabelRow>> {
        unimplemented_repo()
    }
    async fn get_label(&mut self, _id: Uuid) -> AppResult<Option<InitiativeLabelRow>> {
        unimplemented_repo()
    }
    async fn find_labels_by_name(&mut self, _name: &str) -> AppResult<Vec<InitiativeLabelRow>> {
        unimplemented_repo()
    }
    async fn set_labels(
        &mut self,
        _id: InitiativeId,
        _labels: &[InitiativeLabelRow],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- Status-Updates -----------------------------------------------------------------------
    async fn insert_update(&mut self, _u: &NewInitiativeUpdate) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get_update(&mut self, _id: InitiativeUpdateId) -> AppResult<Option<InitiativeUpdate>> {
        unimplemented_repo()
    }
    async fn latest_update(
        &mut self,
        _initiative: InitiativeId,
        _except: Option<InitiativeUpdateId>,
    ) -> AppResult<Option<InitiativeUpdate>> {
        unimplemented_repo()
    }
    async fn list_updates(
        &mut self,
        _filter: &InitiativeUpdateFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<InitiativeUpdate>> {
        unimplemented_repo()
    }
    async fn edit_update(
        &mut self,
        _id: InitiativeUpdateId,
        _body: Option<&str>,
        _health: Option<Health>,
        _now: DateTime<Utc>,
    ) -> AppResult<InitiativeUpdate> {
        unimplemented_repo()
    }
    async fn archive_update(
        &mut self,
        _id: InitiativeUpdateId,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn update_comment_count(&mut self, _id: InitiativeUpdateId) -> AppResult<i64> {
        unimplemented_repo()
    }

    // --- Erinnerungen ---------------------------------------------------------------------------
    /// Initiativen im Status `Active` mit Owner (nicht archiviert/getrasht).
    async fn reminder_targets(&mut self) -> AppResult<Vec<ReminderTarget>> {
        unimplemented_repo()
    }
    /// Erinnerungseinstellungen des Workspace dieser Transaktion.
    async fn reminder_settings(&mut self) -> AppResult<Option<ReminderSettings>> {
        unimplemented_repo()
    }
}
