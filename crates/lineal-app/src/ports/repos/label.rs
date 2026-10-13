//! Repository-Port `LabelRepo`: Issue-, Projekt- und Initiative-Labels (docs/03 §5, docs/07 §3.4).
//!
//! Die drei Familien liegen in drei Tabellen mit fast gleichem Aufbau; `LabelKind` wählt die Tabelle.
//! Initiative-Labels haben keinen Team-Scope, Projekt-/Initiative-Gruppen sind immer `multiSelect`.

use super::user::{Page, PageRequest};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::LabelGroupType;
use lineal_domain::ids::{IssueId, LabelId, TeamId, UserId};

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Label-Familie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelKind {
    Issue,
    Project,
    Initiative,
}

impl LabelKind {
    /// Name der Entität in Fehlermeldungen.
    pub const fn noun(&self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Project => "project",
            Self::Initiative => "initiative",
        }
    }
}

/// Ein Label.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub id: LabelId,
    pub kind: LabelKind,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    /// `None` = Workspace-Label.
    pub team_id: Option<TeamId>,
    pub parent_id: Option<LabelId>,
    pub is_group: bool,
    /// Nur bei Gruppen; `None` bedeutet `singleSelect` (Issue-Labels) bzw. `multiSelect` (sonst).
    pub group_type: Option<LabelGroupType>,
    pub inherited_from_id: Option<LabelId>,
    pub creator_id: Option<UserId>,
    pub retired_at: Option<DateTime<Utc>>,
    pub retired_by_id: Option<UserId>,
    pub last_applied_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Label {
    /// Wirksamer Gruppentyp.
    pub fn effective_group_type(&self) -> LabelGroupType {
        match (self.kind, self.group_type) {
            (LabelKind::Issue, Some(t)) => t,
            (LabelKind::Issue, None) => LabelGroupType::SingleSelect,
            _ => LabelGroupType::MultiSelect,
        }
    }
}

/// Neues Label.
#[derive(Debug, Clone, PartialEq)]
pub struct NewLabel {
    pub id: LabelId,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    pub team_id: Option<TeamId>,
    pub parent_id: Option<LabelId>,
    pub is_group: bool,
    pub group_type: Option<LabelGroupType>,
    pub creator_id: Option<UserId>,
    pub now: DateTime<Utc>,
}

/// Änderung eines Labels (`None` = unverändert; `Some(None)` leert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<Option<String>>,
    pub parent_id: Option<Option<LabelId>>,
    pub is_group: Option<bool>,
    pub group_type: Option<LabelGroupType>,
}

/// Filter der Label-Liste.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelListFilter {
    /// Gewünschte Scopes: `None` im Vec = Workspace-Labels, `Some(team)` = Labels dieses Teams.
    /// Der Aufrufer begrenzt auf sichtbare Teams; leer = nur Workspace-Labels sind NICHT gemeint,
    /// sondern „keine Treffer“ (der Aufrufer füllt immer mindestens `None`).
    pub scopes: Vec<Option<TeamId>>,
    /// Teilstring im Namen, case-insensitiv.
    pub name: Option<String>,
    pub include_groups: bool,
    pub include_archived: bool,
}

/// Persistenzzugriff des Bereichs `label`.
#[async_trait]
pub trait LabelRepo: Send {
    async fn insert(&mut self, _kind: LabelKind, _label: &NewLabel) -> AppResult<Label> {
        unimplemented_repo()
    }
    async fn get(&mut self, _kind: LabelKind, _id: LabelId) -> AppResult<Option<Label>> {
        unimplemented_repo()
    }
    async fn get_many(&mut self, _kind: LabelKind, _ids: &[LabelId]) -> AppResult<Vec<Label>> {
        unimplemented_repo()
    }
    /// Alle nicht archivierten Labels, deren Name case-insensitiv `name` ist.
    async fn find_by_name(&mut self, _kind: LabelKind, _name: &str) -> AppResult<Vec<Label>> {
        unimplemented_repo()
    }
    /// Kinder einer Gruppe (auch retired, nicht archiviert).
    async fn children(&mut self, _kind: LabelKind, _group: LabelId) -> AppResult<Vec<Label>> {
        unimplemented_repo()
    }
    async fn list(
        &mut self,
        _kind: LabelKind,
        _filter: &LabelListFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<Label>> {
        unimplemented_repo()
    }
    /// Wendet den Patch an (`version + 1`, `updated_at = now`).
    async fn update(
        &mut self,
        _kind: LabelKind,
        _id: LabelId,
        _patch: &LabelPatch,
        _now: DateTime<Utc>,
    ) -> AppResult<Label> {
        unimplemented_repo()
    }
    /// Setzt bzw. leert `retired_at` bei allen genannten Labels.
    async fn set_retired(
        &mut self,
        _kind: LabelKind,
        _ids: &[LabelId],
        _retired_by: Option<UserId>,
        _retired: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Anzahl Kinder einer Gruppe (für das Limit von 250).
    async fn count_children(&mut self, _kind: LabelKind, _group: LabelId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Issues (id, Team), die eines der Issue-Labels tragen (WP33: Merge/Delete).
    async fn issues_with_issue_labels(
        &mut self,
        _labels: &[LabelId],
    ) -> AppResult<Vec<(IssueId, TeamId)>> {
        unimplemented_repo()
    }
    /// Wie [`LabelRepo::issues_with_issue_labels`], ohne Issues im Papierkorb (Anzeige der Nutzung).
    async fn active_issues_with_issue_labels(
        &mut self,
        _labels: &[LabelId],
    ) -> AppResult<Vec<(IssueId, TeamId)>> {
        unimplemented_repo()
    }
    /// Hängt alle Issues von `source` an `target` um (ohne Duplikate) und ersetzt die ID in
    /// Templates und gespeicherten Views; `source` bleibt bestehen.
    async fn merge_issue_label(&mut self, _source: LabelId, _target: LabelId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Löscht Issue-Labels endgültig (Zuordnungen entfallen, geerbte Kopien verlieren den Bezug).
    async fn delete_issue_labels(&mut self, _labels: &[LabelId]) -> AppResult<()> {
        unimplemented_repo()
    }
}
