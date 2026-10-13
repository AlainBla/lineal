//! Repository-Port `IssueRepo`: Issues, Labels-Zuordnung, Relationen, History, Aliase, Sharing
//! (docs/03 §2–5, §8–9, docs/02 §7).
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank
//! kompilieren. `identifier` und `state_type` pflegt die Datenbank per Trigger; beim Einfügen
//! werden die Werte der Struktur ignoriert und die der Datenbank zurückgeliefert.

use super::user::{Page, PageRequest};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::{Priority, RelationType, SlaType, StateType};
use lineal_domain::ids::{
    CycleId, IssueId, IssueRelationId, LabelId, MilestoneId, OrgId, ProjectId, StateId, TeamId,
    UserId,
};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Issue (eine Zeile der Tabelle `issue`).
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub id: IssueId,
    pub org_id: OrgId,
    pub team_id: TeamId,
    pub number: i32,
    /// Denormalisierte Kennung `KEY-n` (Trigger).
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub state_id: StateId,
    /// Denormalisierte Kategorie des Status (Trigger).
    pub state_type: StateType,
    pub state_entered_at: DateTime<Utc>,
    pub priority: Priority,
    pub estimate: Option<i32>,
    pub assignee_id: Option<UserId>,
    pub delegate_id: Option<UserId>,
    pub creator_id: Option<UserId>,
    pub parent_id: Option<IssueId>,
    pub project_id: Option<ProjectId>,
    pub project_milestone_id: Option<MilestoneId>,
    pub cycle_id: Option<CycleId>,
    pub due_date: Option<NaiveDate>,
    pub sort_order: f64,
    pub priority_sort_order: f64,
    pub sub_issue_sort_order: Option<f64>,
    pub started_triage_at: Option<DateTime<Utc>>,
    pub triaged_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub auto_closed_at: Option<DateTime<Utc>>,
    pub snoozed_until_at: Option<DateTime<Utc>>,
    pub snoozed_by_id: Option<UserId>,
    pub added_to_team_at: DateTime<Utc>,
    pub added_to_cycle_at: Option<DateTime<Utc>>,
    pub added_to_project_at: Option<DateTime<Utc>>,
    pub sla_type: Option<SlaType>,
    pub sla_started_at: Option<DateTime<Utc>>,
    pub sla_medium_risk_at: Option<DateTime<Utc>>,
    pub sla_high_risk_at: Option<DateTime<Utc>>,
    pub sla_breaches_at: Option<DateTime<Utc>>,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Anzeigewerte, die zu einem Issue gehören (ein Batch-Query, kein N+1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueExtras {
    pub state_name: String,
    pub team_key: String,
    pub team_name: String,
    pub assignee_name: Option<String>,
    pub delegate_name: Option<String>,
    pub creator_name: Option<String>,
    pub project_name: Option<String>,
    /// `P-KEY-n`.
    pub project_identifier: Option<String>,
    pub milestone_name: Option<String>,
    /// Kennung des Parents.
    pub parent_identifier: Option<String>,
    /// Label-Namen (einfacher Name, auch für Labels in Gruppen) in stabiler Reihenfolge.
    pub label_names: Vec<String>,
    pub label_ids: Vec<LabelId>,
    pub children_count: i64,
}

/// Quelle eines History-Eintrags (`issue_history.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistorySource {
    #[default]
    User,
    Automation,
    Api,
    Mcp,
    Import,
    System,
}

impl HistorySource {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Automation => "automation",
            Self::Api => "api",
            Self::Mcp => "mcp",
            Self::Import => "import",
            Self::System => "system",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "automation" => Self::Automation,
            "api" => Self::Api,
            "mcp" => Self::Mcp,
            "import" => Self::Import,
            "system" => Self::System,
            _ => Self::User,
        }
    }
}

/// Ein History-Eintrag (append-only; Zusammenfassen = löschen + neu schreiben).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub issue_id: IssueId,
    pub actor_id: Option<UserId>,
    pub source: HistorySource,
    /// `{ "<feld>": { "from": …, "to": … } }` mit camelCase-Feldnamen.
    pub changes: Value,
    pub added_label_ids: Option<Vec<LabelId>>,
    pub removed_label_ids: Option<Vec<LabelId>>,
    /// `[{ "type", "identifier", "action": "added"|"removed" }]`.
    pub relation_changes: Option<Value>,
    pub flags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Eine Relation (eine Zeile; `related`/`similar` kanonisch mit kleinerer UUID als `issue_id`).
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub id: IssueRelationId,
    pub issue_id: IssueId,
    pub related_issue_id: IssueId,
    pub relation_type: RelationType,
    pub creator_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// Ein Zyklus (nur das, was die Zuordnung braucht).
#[derive(Debug, Clone, PartialEq)]
pub struct CycleLookup {
    pub id: CycleId,
    pub team_id: TeamId,
    pub number: i32,
    pub name: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub inherited_from_id: Option<CycleId>,
}

/// Ein Projekt (nur das, was die Zuordnung braucht).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectLookup {
    pub id: ProjectId,
    pub name: String,
    pub identifier: String,
    pub team_ids: Vec<TeamId>,
    pub trashed: bool,
}

/// Ein Meilenstein (nur das, was die Zuordnung braucht).
#[derive(Debug, Clone, PartialEq)]
pub struct MilestoneLookup {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: String,
}

/// Filter von [`IssueRepo::list`]. Der Aufrufer begrenzt `team_ids` immer auf sichtbare Teams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueListFilter {
    pub team_ids: Vec<TeamId>,
    /// `Some(None)` = ohne Assignee.
    pub assignee: Option<Option<UserId>>,
    pub creator: Option<UserId>,
    pub delegate: Option<UserId>,
    pub state_ids: Option<Vec<StateId>>,
    pub state_types: Option<Vec<StateType>>,
    pub label_ids: Option<Vec<LabelId>>,
    pub project_id: Option<ProjectId>,
    pub cycle_id: Option<CycleId>,
    pub parent_id: Option<IssueId>,
    pub priority: Option<Priority>,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub triaged_after: Option<DateTime<Utc>>,
    pub include_archived: bool,
    /// `Some(t)`: stattdessen nur Issues im Papierkorb, die seit `t` gelöscht wurden („Recently
    /// deleted“); der Archiv-Filter entfällt (Gelöschte sind zugleich archiviert).
    pub trashed_since: Option<DateTime<Utc>>,
    /// Nur Issues, die mit diesen IDs geteilt sind, zusätzlich zu `team_ids` (Sharing).
    pub shared_with: Option<UserId>,
    /// Filterbaum (Custom View, Ad-hoc-Filter; WP11). Setzt `eval` voraus. Wird im Adapter
    /// parametrisiert nach SQL übersetzt und immer per UND mit der Sichtbarkeit verknüpft.
    pub ast: Option<Box<lineal_domain::rules::filter::IssueFilter>>,
    /// Auswertungskontext für `ast` (Zeitpunkt, Zeitzone, „ich“, sichtbare Teams).
    pub eval: Option<super::view::FilterEval>,
    /// Volltext-/Kennungssuche (`list_issues.query`, WP11): höchstens 500 Treffer.
    pub search: Option<String>,
}

/// Zugriff eines Users auf ein einzelnes Issue (Sharing, docs/02).
#[derive(Debug, Clone, PartialEq)]
pub struct IssueShare {
    pub issue_id: IssueId,
    pub user_id: UserId,
    pub shared_by_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// Persistenzzugriff des Bereichs `issue`.
#[async_trait]
pub trait IssueRepo: Send {
    /// Fügt ein Issue ein (Nummer vorher per `allocate_issue_numbers` reserviert). `identifier`
    /// und `state_type` setzt die Datenbank; zurück kommt die gespeicherte Zeile.
    async fn insert(&mut self, _issue: &Issue) -> AppResult<Issue> {
        unimplemented_repo()
    }
    /// Lädt ein Issue (auch archivierte und gelöschte); `lock` = `FOR UPDATE`.
    async fn get(&mut self, _id: IssueId, _lock: bool) -> AppResult<Option<Issue>> {
        unimplemented_repo()
    }
    /// Schreibt alle veränderlichen Felder zurück (`version + 1`, `updated_at = now`).
    async fn save(&mut self, _issue: &Issue, _now: DateTime<Utc>) -> AppResult<Issue> {
        unimplemented_repo()
    }
    /// Issue mit dieser Nummer im Team.
    async fn find_by_number(&mut self, _team: TeamId, _number: i32) -> AppResult<Option<IssueId>> {
        unimplemented_repo()
    }
    /// Issue, das früher diese Kennung hatte (`issue_identifier_alias`).
    async fn find_by_alias(&mut self, _identifier: &str) -> AppResult<Option<IssueId>> {
        unimplemented_repo()
    }
    /// Gefilterte Seite, absteigend nach `updated_at`/`created_at` und `id` (Papierkorb nie).
    async fn list(
        &mut self,
        _filter: &IssueListFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<Issue>> {
        unimplemented_repo()
    }
    /// Anzeigewerte für mehrere Issues.
    async fn extras(&mut self, _ids: &[IssueId]) -> AppResult<HashMap<IssueId, IssueExtras>> {
        unimplemented_repo()
    }
    /// Alle früheren Kennungen (Aliase und frühere Team-Keys mit der aktuellen Nummer).
    async fn previous_identifiers(&mut self, _issue: IssueId) -> AppResult<Vec<String>> {
        unimplemented_repo()
    }
    /// Offene (nicht archivierte, nicht gelöschte) Issues des Teams.
    async fn count_open(&mut self, _team: TeamId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Kleinster und größter `sort_order` der aktiven Issues im Status.
    async fn sort_extent(
        &mut self,
        _team: TeamId,
        _state: StateId,
    ) -> AppResult<(Option<f64>, Option<f64>)> {
        unimplemented_repo()
    }
    /// Größter `sub_issue_sort_order` der Children eines Parents.
    async fn max_sub_issue_sort_order(&mut self, _parent: IssueId) -> AppResult<Option<f64>> {
        unimplemented_repo()
    }

    // --- Hierarchie ---
    /// Vorfahren (nächster zuerst, höchstens 64).
    async fn ancestor_ids(&mut self, _issue: IssueId) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Tiefe des Teilbaums unterhalb des Issues (ohne Children = 0).
    async fn descendant_depth(&mut self, _issue: IssueId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Direkte Children (nicht im Papierkorb).
    async fn children(&mut self, _issue: IssueId) -> AppResult<Vec<Issue>> {
        unimplemented_repo()
    }

    // --- Labels ---
    async fn label_ids(&mut self, _issue: IssueId) -> AppResult<Vec<LabelId>> {
        unimplemented_repo()
    }
    /// Entfernt `remove` und fügt `add` (`(Label, Gruppe bei singleSelect)`) hinzu; setzt
    /// `last_applied_at` der hinzugefügten Labels.
    async fn apply_labels(
        &mut self,
        _issue: IssueId,
        _remove: &[LabelId],
        _add: &[(LabelId, Option<LabelId>)],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- History ---
    async fn insert_history(&mut self, _entry: &HistoryEntry) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn last_history(&mut self, _issue: IssueId) -> AppResult<Option<HistoryEntry>> {
        unimplemented_repo()
    }
    /// Löscht einen Eintrag (nur für das Zusammenfassen: danach wird der verrechnete neu geschrieben).
    async fn delete_history(&mut self, _id: Uuid) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Einträge eines Issues, älteste zuerst.
    async fn list_history(&mut self, _issue: IssueId, _limit: i64) -> AppResult<Vec<HistoryEntry>> {
        unimplemented_repo()
    }

    // --- Relationen ---
    /// Legt die Relation an; `false`, wenn sie (im kanonischen Sinn) schon existiert.
    async fn insert_relation(
        &mut self,
        _id: IssueRelationId,
        _issue: IssueId,
        _related: IssueId,
        _kind: RelationType,
        _creator: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Löscht die Relation (`blocks`/`duplicate` gerichtet, `related`/`similar` symmetrisch);
    /// liefert die gelöschte Zeile.
    async fn delete_relation(
        &mut self,
        _issue: IssueId,
        _related: IssueId,
        _kind: RelationType,
    ) -> AppResult<Option<Relation>> {
        unimplemented_repo()
    }
    /// Alle Relationen, an denen das Issue beteiligt ist (beide Richtungen).
    async fn relations_of(&mut self, _issue: IssueId) -> AppResult<Vec<Relation>> {
        unimplemented_repo()
    }
    async fn get_relation(&mut self, _id: IssueRelationId) -> AppResult<Option<Relation>> {
        unimplemented_repo()
    }
    async fn delete_relation_by_id(&mut self, _id: IssueRelationId) -> AppResult<bool> {
        unimplemented_repo()
    }

    // --- Aliase (Teamwechsel) ---
    async fn insert_alias(
        &mut self,
        _issue: IssueId,
        _identifier: &str,
        _team_key: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Alias dieses Issues mit einem dieser Team-Keys: `(Kennung, Nummer)`.
    async fn find_alias_for_keys(
        &mut self,
        _issue: IssueId,
        _keys: &[String],
    ) -> AppResult<Option<(String, i32)>> {
        unimplemented_repo()
    }
    async fn delete_alias(&mut self, _identifier: &str) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Frühere Keys des Teams (`team_key`).
    async fn team_key_history(&mut self, _team: TeamId) -> AppResult<Vec<String>> {
        unimplemented_repo()
    }

    // --- Referenzen für die Zuordnung (WP07 besitzt die Planungs-Repos; das hier sind
    //     reine Lesezugriffe für die Validierung) ---
    /// Cycle des Teams: UUID, Nummer oder Name (case-insensitiv); mehrere Treffer = mehrdeutig.
    async fn find_cycles(
        &mut self,
        _team: TeamId,
        _reference: &str,
    ) -> AppResult<Vec<CycleLookup>> {
        unimplemented_repo()
    }
    /// Cycle des Teams, der vom Parent-Cycle `root` erbt (oder `root` selbst ist), nicht abgeschlossen.
    async fn find_cycle_inheriting(
        &mut self,
        _team: TeamId,
        _root: CycleId,
    ) -> AppResult<Option<CycleLookup>> {
        unimplemented_repo()
    }
    /// Cycle per ID (beliebiges Team).
    async fn get_cycle(&mut self, _id: CycleId) -> AppResult<Option<CycleLookup>> {
        unimplemented_repo()
    }
    /// Projekte: UUID, `P-KEY-n`, Slug oder exakter Name.
    async fn find_projects(&mut self, _reference: &str) -> AppResult<Vec<ProjectLookup>> {
        unimplemented_repo()
    }
    async fn get_project(&mut self, _id: ProjectId) -> AppResult<Option<ProjectLookup>> {
        unimplemented_repo()
    }
    /// Meilensteine eines Projekts: UUID oder exakter Name.
    async fn find_milestones(
        &mut self,
        _project: ProjectId,
        _reference: &str,
    ) -> AppResult<Vec<MilestoneLookup>> {
        unimplemented_repo()
    }

    // --- Sharing ---
    async fn insert_share(&mut self, _share: &IssueShare) -> AppResult<bool> {
        unimplemented_repo()
    }
    async fn delete_share(&mut self, _issue: IssueId, _user: UserId) -> AppResult<bool> {
        unimplemented_repo()
    }
    async fn get_share(&mut self, _issue: IssueId, _user: UserId) -> AppResult<Option<IssueShare>> {
        unimplemented_repo()
    }

    /// Löscht das Issue endgültig (History, Relationen, Labels, Anhänge fallen per FK mit).
    async fn delete(&mut self, _id: IssueId) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- Abos ---
    /// Abonniert (idempotent; reaktiviert ein abbestelltes Abo nicht).
    async fn subscribe(
        &mut self,
        _id: Uuid,
        _issue: IssueId,
        _user: UserId,
        _reason: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
}
