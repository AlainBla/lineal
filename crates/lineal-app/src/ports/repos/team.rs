//! Repository-Port `TeamRepo`: Teams, Key-Historie, Mitgliedschaften und Zähler.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne
//! Datenbank kompilieren.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{EstimationType, TeamVisibility};
use lineal_domain::ids::{IssueId, OrgId, StateId, TeamId, UserId};
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Team mit allen Einstellungen (docs/02 §2.5). Hierarchie (`displayName`, Vorfahren)
/// wird aus [`TeamRepo::list_all`] im Use-Case berechnet (siehe `policy::TeamTree`).
#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub id: TeamId,
    pub org_id: OrgId,
    pub name: String,
    pub key: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub parent_id: Option<TeamId>,
    pub visibility: TeamVisibility,
    pub all_members_can_join: bool,
    pub timezone: String,
    /// `Team.securitySettings` (`teamManagement`, `labelManagement`, … je `member`|`owner`).
    pub security_settings: Value,
    pub retired_at: Option<DateTime<Utc>>,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    /// Zähler: zuletzt vergebene Nummer.
    pub last_issue_number: i32,
    pub last_project_number: i32,
    pub last_cycle_number: i32,
    pub default_issue_state_id: Option<StateId>,
    pub triage_enabled: bool,
    pub triage_issue_state_id: Option<StateId>,
    pub require_priority_to_leave_triage: bool,
    pub auto_close_period: Option<i32>,
    pub auto_close_state_id: Option<StateId>,
    pub auto_archive_period: i32,
    pub auto_close_parent_issues: bool,
    pub auto_close_child_issues: bool,
    pub set_issue_sort_order_on_state_change: String,
    pub group_issue_history: bool,
    pub inherit_workflow_statuses: bool,
    pub issue_estimation_type: EstimationType,
    pub issue_estimation_extended: bool,
    pub issue_estimation_allow_zero: bool,
    pub default_issue_estimate: i32,
    pub inherit_issue_estimation: bool,
    pub cycles_enabled: bool,
    pub cycle_duration: i32,
    pub cycle_cooldown_time: i32,
    pub cycle_start_day: i32,
    pub upcoming_cycle_count: i32,
    pub cycle_issue_auto_assign_started: bool,
    pub cycle_issue_auto_assign_completed: bool,
    pub cycle_lock_to_active: bool,
    pub inherit_project_statuses: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neues Team (Defaults der übrigen Spalten setzt die Datenbank bzw. der Use-Case per Patch).
#[derive(Debug, Clone, PartialEq)]
pub struct NewTeam {
    pub id: TeamId,
    pub name: String,
    pub key: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub parent_id: Option<TeamId>,
    pub visibility: TeamVisibility,
    pub all_members_can_join: bool,
    pub timezone: String,
    pub security_settings: Value,
    pub triage_enabled: bool,
    pub inherit_workflow_statuses: bool,
    pub inherit_issue_estimation: bool,
    pub now: DateTime<Utc>,
}

impl Team {
    /// Baut ein `Team` mit den Spalten-Defaults der Datenbank (für Fakes und Tests).
    pub fn from_new(org_id: OrgId, t: &NewTeam) -> Team {
        Team {
            id: t.id,
            org_id,
            name: t.name.clone(),
            key: t.key.clone(),
            description: t.description.clone(),
            icon: t.icon.clone(),
            color: t.color.clone(),
            parent_id: t.parent_id,
            visibility: t.visibility,
            all_members_can_join: t.all_members_can_join,
            timezone: t.timezone.clone(),
            security_settings: t.security_settings.clone(),
            retired_at: None,
            trashed: false,
            trashed_at: None,
            last_issue_number: 0,
            last_project_number: 0,
            last_cycle_number: 0,
            default_issue_state_id: None,
            triage_enabled: t.triage_enabled,
            triage_issue_state_id: None,
            require_priority_to_leave_triage: false,
            auto_close_period: None,
            auto_close_state_id: None,
            auto_archive_period: 6,
            auto_close_parent_issues: false,
            auto_close_child_issues: false,
            set_issue_sort_order_on_state_change: "top".into(),
            group_issue_history: true,
            inherit_workflow_statuses: t.inherit_workflow_statuses,
            issue_estimation_type: EstimationType::NotUsed,
            issue_estimation_extended: false,
            issue_estimation_allow_zero: false,
            default_issue_estimate: 1,
            inherit_issue_estimation: t.inherit_issue_estimation,
            cycles_enabled: false,
            cycle_duration: 2,
            cycle_cooldown_time: 0,
            cycle_start_day: 1,
            upcoming_cycle_count: 2,
            cycle_issue_auto_assign_started: false,
            cycle_issue_auto_assign_completed: false,
            cycle_lock_to_active: false,
            inherit_project_statuses: true,
            version: 1,
            created_at: t.now,
            updated_at: t.now,
        }
    }
}

/// Änderung eines Teams. `None` = unverändert; `Some(None)` leert nullbare Felder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamPatch {
    pub name: Option<String>,
    pub key: Option<String>,
    pub description: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub parent_id: Option<Option<TeamId>>,
    pub visibility: Option<TeamVisibility>,
    pub all_members_can_join: Option<bool>,
    pub timezone: Option<String>,
    pub security_settings: Option<Value>,
    pub default_issue_state_id: Option<Option<StateId>>,
    pub triage_enabled: Option<bool>,
    pub triage_issue_state_id: Option<Option<StateId>>,
    pub require_priority_to_leave_triage: Option<bool>,
    pub auto_close_period: Option<Option<i32>>,
    pub auto_close_state_id: Option<Option<StateId>>,
    pub auto_archive_period: Option<i32>,
    pub auto_close_parent_issues: Option<bool>,
    pub auto_close_child_issues: Option<bool>,
    pub set_issue_sort_order_on_state_change: Option<String>,
    pub group_issue_history: Option<bool>,
    pub inherit_workflow_statuses: Option<bool>,
    pub issue_estimation_type: Option<EstimationType>,
    pub issue_estimation_extended: Option<bool>,
    pub issue_estimation_allow_zero: Option<bool>,
    pub default_issue_estimate: Option<i32>,
    pub inherit_issue_estimation: Option<bool>,
    pub cycles_enabled: Option<bool>,
    pub cycle_duration: Option<i32>,
    pub cycle_cooldown_time: Option<i32>,
    pub cycle_start_day: Option<i32>,
    pub upcoming_cycle_count: Option<i32>,
    pub cycle_issue_auto_assign_started: Option<bool>,
    pub cycle_issue_auto_assign_completed: Option<bool>,
    pub cycle_lock_to_active: Option<bool>,
    pub inherit_project_statuses: Option<bool>,
}

/// Mitgliedschaft eines Users in einem Team.
#[derive(Debug, Clone, PartialEq)]
pub struct Membership {
    pub id: Uuid,
    pub team_id: TeamId,
    pub user_id: UserId,
    pub owner: bool,
    pub sort_order: f64,
    pub created_at: DateTime<Utc>,
}

/// Neue Mitgliedschaft.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMembership {
    pub id: Uuid,
    pub team_id: TeamId,
    pub user_id: UserId,
    pub owner: bool,
    pub now: DateTime<Utc>,
}

/// Persistenzzugriff des Bereichs `team`.
#[async_trait]
pub trait TeamRepo: Send {
    async fn insert(&mut self, _team: &NewTeam) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: TeamId) -> AppResult<Option<Team>> {
        unimplemented_repo()
    }
    /// Alle Teams des Workspace (auch stillgelegte und gelöschte); der Aufrufer filtert.
    async fn list_all(&mut self) -> AppResult<Vec<Team>> {
        unimplemented_repo()
    }
    /// Wendet den Patch an (`version + 1`, `updated_at = now`) und liefert den neuen Stand.
    async fn update(
        &mut self,
        _id: TeamId,
        _patch: &TeamPatch,
        _now: DateTime<Utc>,
    ) -> AppResult<Team> {
        unimplemented_repo()
    }

    /// Ob `key` von einem anderen Team als `except` verwendet wird (aktiv oder historisch).
    async fn key_reserved(&mut self, _key: &str, _except: Option<TeamId>) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Team zu einem Key (aktiver Key zuerst, dann Key-Historie).
    async fn find_id_by_key(&mut self, _key: &str) -> AppResult<Option<TeamId>> {
        unimplemented_repo()
    }
    /// Notiert einen früheren Key des Teams (`team_key` enthält nur frühere Keys).
    async fn insert_key_history(
        &mut self,
        _team: TeamId,
        _key: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Entfernt einen Eintrag der Key-Historie (beim Reaktivieren eines alten Keys).
    async fn remove_key_history(&mut self, _team: TeamId, _key: &str) -> AppResult<()> {
        unimplemented_repo()
    }

    async fn list_memberships(&mut self, _team: TeamId) -> AppResult<Vec<Membership>> {
        unimplemented_repo()
    }
    async fn list_memberships_of_user(&mut self, _user: UserId) -> AppResult<Vec<Membership>> {
        unimplemented_repo()
    }
    async fn get_membership(
        &mut self,
        _team: TeamId,
        _user: UserId,
    ) -> AppResult<Option<Membership>> {
        unimplemented_repo()
    }
    async fn insert_membership(&mut self, _m: &NewMembership) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn set_membership_owner(
        &mut self,
        _team: TeamId,
        _user: UserId,
        _owner: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Entfernt die Mitgliedschaft; `true`, wenn eine existierte.
    async fn delete_membership(&mut self, _team: TeamId, _user: UserId) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Räumt nach dem Austritt von `user` aus den (nicht öffentlichen) `teams` auf (docs/02 §4.2):
    /// `assignee = null` bei offenen Issues dieser Teams (liefert deren IDs für die History),
    /// Issue-Abos auf Issues dieser Teams werden entfernt, und ein Projekt-Lead ohne verbleibenden
    /// Zugriff (kein Projektteam öffentlich oder mit Mitgliedschaft) wird geleert.
    async fn release_member_assets(
        &mut self,
        _teams: &[TeamId],
        _user: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }

    /// Reserviert `count` Issue-Nummern atomar (`UPDATE team SET last_issue_number = … RETURNING`)
    /// und liefert die **erste** reservierte Nummer; die Nummern sind `first .. first + count`.
    /// `not_found`, wenn das Team nicht existiert, gelöscht oder stillgelegt ist.
    /// Muss in derselben Transaktion wie das Anlegen des Issues laufen.
    async fn allocate_issue_numbers(&mut self, _team: TeamId, _count: i32) -> AppResult<i32> {
        unimplemented_repo()
    }
    /// Nächste Projektnummer (Zähler des Lead-Teams), atomar.
    async fn next_project_number(&mut self, _team: TeamId) -> AppResult<i32> {
        unimplemented_repo()
    }
    /// Nächste Cycle-Nummer, atomar.
    async fn next_cycle_number(&mut self, _team: TeamId) -> AppResult<i32> {
        unimplemented_repo()
    }
}
