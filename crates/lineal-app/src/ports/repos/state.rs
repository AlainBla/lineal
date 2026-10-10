//! Repository-Port `WorkflowStateRepo`: Workflow-Status eines Teams (docs/03 §1).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::StateType;
use lineal_domain::ids::{OrgId, StateId, TeamId, UserId};

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Workflow-Status.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowState {
    pub id: StateId,
    pub org_id: OrgId,
    pub team_id: TeamId,
    pub name: String,
    pub state_type: StateType,
    pub color: String,
    pub description: Option<String>,
    pub position: f64,
    pub inherited_from_id: Option<StateId>,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Status.
#[derive(Debug, Clone, PartialEq)]
pub struct NewState {
    pub id: StateId,
    pub team_id: TeamId,
    pub name: String,
    pub state_type: StateType,
    pub color: String,
    pub description: Option<String>,
    pub position: f64,
    pub inherited_from_id: Option<StateId>,
    pub now: DateTime<Utc>,
}

/// Änderung (`type` ist unveränderlich).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatePatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<Option<String>>,
    pub position: Option<f64>,
    /// Löst die Verknüpfung zum Parent-Status (`Some(None)`), z. B. beim Un-Nesting.
    pub inherited_from: Option<Option<StateId>>,
}

/// Persistenzzugriff des Bereichs `state`.
#[async_trait]
pub trait WorkflowStateRepo: Send {
    async fn insert(&mut self, _state: &NewState) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: StateId) -> AppResult<Option<WorkflowState>> {
        unimplemented_repo()
    }
    /// Status eines Teams, sortiert nach Kategorie-Reihenfolge und `position`.
    async fn list_for_team(
        &mut self,
        _team: TeamId,
        _include_archived: bool,
    ) -> AppResult<Vec<WorkflowState>> {
        unimplemented_repo()
    }
    async fn update(
        &mut self,
        _id: StateId,
        _patch: &StatePatch,
        _now: DateTime<Utc>,
    ) -> AppResult<WorkflowState> {
        unimplemented_repo()
    }
    /// Archiviert den Status (`archived_at = now`).
    async fn archive(&mut self, _id: StateId, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Anzahl nicht archivierter Status einer Kategorie im Team.
    async fn count_active_of_type(
        &mut self,
        _team: TeamId,
        _state_type: StateType,
    ) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Anzahl Issues (inkl. archivierter und gelöschter) mit diesem Status.
    async fn count_issues(&mut self, _id: StateId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Setzt `from` → `to` bei allen Issues (Kategorie-Zeitstempel nach docs/03 §1.6,
    /// `version + 1`, je ein `issue_history`-Eintrag mit `actor`); liefert die Anzahl.
    /// Beide Status gehören zum selben Team.
    async fn migrate_issues(
        &mut self,
        _from: StateId,
        _to: StateId,
        _actor: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }
    /// Kopien (`inherited_from_id = source`) in anderen Teams.
    async fn list_copies_of(&mut self, _source: StateId) -> AppResult<Vec<WorkflowState>> {
        unimplemented_repo()
    }
    /// Aktiver Status des Teams mit diesem Namen (case-insensitiv).
    async fn find_by_name(
        &mut self,
        _team: TeamId,
        _name: &str,
    ) -> AppResult<Option<WorkflowState>> {
        unimplemented_repo()
    }
}
