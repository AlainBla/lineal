//! Repository-Port `ImportRepo` (WP18, docs/13).
//!
//! Der Import schreibt über **import-spezifische** Methoden, weil die Use-Cases (`create_issue_in`
//! u. a.) Policy, Triage, Automationen und Nummernvergabe mitbringen und weder Linear-`number`,
//! `createdAt`, Autor noch `EventOrigin::Import` zulassen. Zeilen werden als JSON-Objekte
//! (`Spaltenname -> Wert`) übergeben; die Spaltennamen kommen ausschließlich vom Import-Code
//! (nicht von Benutzern) und werden zusätzlich gegen `^[a-z_]+$` geprüft.
//!
//! Alle Methoden haben Default-Rümpfe, damit Fake-Transaktionen ohne Import auskommen.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

fn nyi<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Tabellen, in die der Import schreibt (Whitelist; `name()` ist der SQL-Name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportTable {
    User,
    Team,
    TeamMembership,
    WorkflowState,
    IssueLabel,
    ProjectStatus,
    Template,
    Cycle,
    Initiative,
    InitiativeParent,
    InitiativeToProject,
    Project,
    ProjectTeam,
    ProjectMember,
    ProjectMilestone,
    ProjectLabel,
    ProjectToLabel,
    Issue,
    IssueToLabel,
    IssueRelation,
    IssueHistory,
    IssueIdentifierAlias,
    IssueSubscription,
    Comment,
    Reaction,
    Attachment,
    Document,
    DocumentContent,
    ProjectUpdate,
    InitiativeUpdate,
    EntityExternalLink,
    CustomView,
    Favorite,
}

impl ImportTable {
    /// SQL-Tabellenname.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::User => "\"user\"",
            Self::Team => "team",
            Self::TeamMembership => "team_membership",
            Self::WorkflowState => "workflow_state",
            Self::IssueLabel => "issue_label",
            Self::ProjectStatus => "project_status",
            Self::Template => "template",
            Self::Cycle => "cycle",
            Self::Initiative => "initiative",
            Self::InitiativeParent => "initiative_parent",
            Self::InitiativeToProject => "initiative_to_project",
            Self::Project => "project",
            Self::ProjectTeam => "project_team",
            Self::ProjectMember => "project_member",
            Self::ProjectMilestone => "project_milestone",
            Self::ProjectLabel => "project_label",
            Self::ProjectToLabel => "project_to_label",
            Self::Issue => "issue",
            Self::IssueToLabel => "issue_to_label",
            Self::IssueRelation => "issue_relation",
            Self::IssueHistory => "issue_history",
            Self::IssueIdentifierAlias => "issue_identifier_alias",
            Self::IssueSubscription => "issue_subscription",
            Self::Comment => "comment",
            Self::Reaction => "reaction",
            Self::Attachment => "attachment",
            Self::Document => "document",
            Self::DocumentContent => "document_content",
            Self::ProjectUpdate => "project_update",
            Self::InitiativeUpdate => "initiative_update",
            Self::EntityExternalLink => "entity_external_link",
            Self::CustomView => "custom_view",
            Self::Favorite => "favorite",
        }
    }
}

/// Zähler, die der Import fortschreibt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportCounter {
    /// `team.last_project_number` (Scope = Team-ID des Lead-Teams).
    Project,
    /// `organization.last_initiative_number` (Scope ignoriert).
    Initiative,
}

/// Ein Importlauf (`import_run`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRun {
    pub id: Uuid,
    pub org_id: Uuid,
    pub source: String,
    pub status: String,
    pub options: Value,
    pub checkpoint: Value,
    pub stats: Value,
    pub started_by: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Importlauf.
#[derive(Debug, Clone, PartialEq)]
pub struct NewImportRun {
    pub id: Uuid,
    pub source: String,
    pub status: String,
    pub options: Value,
    pub started_by: Option<Uuid>,
    pub now: DateTime<Utc>,
}

/// Teilaktualisierung eines Laufs (`None` = unverändert; `Some(None)` setzt auf NULL).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportRunUpdate {
    pub status: Option<String>,
    pub options: Option<Value>,
    pub checkpoint: Option<Value>,
    pub stats: Option<Value>,
    pub started_at: Option<Option<DateTime<Utc>>>,
    pub finished_at: Option<Option<DateTime<Utc>>>,
    pub error: Option<Option<String>>,
    pub now: Option<DateTime<Utc>>,
}

/// Zeile aus `import_mapping`.
#[derive(Debug, Clone, PartialEq)]
pub struct MappingRow {
    pub import_run_id: Uuid,
    pub source: String,
    pub source_type: String,
    pub source_id: String,
    pub target_id: Uuid,
    pub source_updated_at: Option<DateTime<Utc>>,
    pub checksum: Option<String>,
    pub imported_at: DateTime<Utc>,
}

/// Neues oder aktualisiertes Mapping (Upsert auf `(organization_id, source, source_type, source_id)`).
#[derive(Debug, Clone, PartialEq)]
pub struct NewMapping {
    pub import_run_id: Uuid,
    pub source: String,
    pub source_type: String,
    pub source_id: String,
    pub target_id: Uuid,
    pub source_updated_at: Option<DateTime<Utc>>,
    pub checksum: Option<String>,
    pub imported_at: DateTime<Utc>,
}

/// Eintrag des Berichts (`import_issue`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReportEntry {
    /// `info`, `warning`, `error`.
    pub severity: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub message: String,
    pub created_at: Option<DateTime<Utc>>,
}

/// Persistenzzugriff des Bereichs `import`.
#[async_trait]
pub trait ImportRepo: Send {
    // --- Läufe ---------------------------------------------------------------------------

    async fn create_import_run(&mut self, run: &NewImportRun) -> AppResult<ImportRun> {
        let _ = run;
        nyi()
    }

    async fn get_import_run(&mut self, id: Uuid) -> AppResult<Option<ImportRun>> {
        let _ = id;
        nyi()
    }

    async fn list_import_runs(&mut self, limit: i64) -> AppResult<Vec<ImportRun>> {
        let _ = limit;
        nyi()
    }

    async fn update_import_run(&mut self, id: Uuid, update: &ImportRunUpdate) -> AppResult<()> {
        let _ = (id, update);
        nyi()
    }

    /// Letzter erfolgreich abgeschlossener, nicht-Dry-Run-Lauf (für das Delta).
    async fn latest_completed_import_run(&mut self, source: &str) -> AppResult<Option<ImportRun>> {
        let _ = source;
        nyi()
    }

    /// Reserviert (mandantenübergreifend, nur `begin_system`) einen ausführbaren Lauf
    /// (`scanning` oder `running`, ohne gültige Lease) und setzt die Lease bis `lease_until`.
    async fn claim_runnable_import(
        &mut self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> AppResult<Option<ImportRun>> {
        let _ = (now, lease_until);
        nyi()
    }

    /// Verlängert (`Some`) oder löscht (`None`) die Lease eines Laufs.
    async fn set_import_lease(
        &mut self,
        id: Uuid,
        lease_until: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        let _ = (id, lease_until);
        nyi()
    }

    // --- Zugangsdaten ----------------------------------------------------------------------

    async fn put_import_credential(
        &mut self,
        run_id: Uuid,
        encrypted: &[u8],
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let _ = (run_id, encrypted, expires_at);
        nyi()
    }

    async fn get_import_credential(&mut self, run_id: Uuid) -> AppResult<Option<Vec<u8>>> {
        let _ = run_id;
        nyi()
    }

    async fn delete_import_credential(&mut self, run_id: Uuid) -> AppResult<()> {
        let _ = run_id;
        nyi()
    }

    /// Löscht abgelaufene Zugangsdaten (mandantenübergreifend, nur `begin_system`).
    async fn delete_expired_import_credentials(&mut self, now: DateTime<Utc>) -> AppResult<u64> {
        let _ = now;
        nyi()
    }

    // --- Mapping ---------------------------------------------------------------------------

    async fn get_mappings(
        &mut self,
        source: &str,
        source_type: &str,
        source_ids: &[String],
    ) -> AppResult<Vec<MappingRow>> {
        let _ = (source, source_type, source_ids);
        nyi()
    }

    /// Mappings eines Typs, aufsteigend nach `source_id`, ab (exklusive) `after`.
    async fn list_mappings(
        &mut self,
        source: &str,
        source_type: &str,
        after: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<MappingRow>> {
        let _ = (source, source_type, after, limit);
        nyi()
    }

    async fn upsert_mapping(&mut self, mapping: &NewMapping) -> AppResult<()> {
        let _ = mapping;
        nyi()
    }

    async fn delete_mapping(
        &mut self,
        source: &str,
        source_type: &str,
        source_id: &str,
    ) -> AppResult<()> {
        let _ = (source, source_type, source_id);
        nyi()
    }

    /// Anzahl Mappings je `source_type`.
    async fn count_mappings(&mut self, source: &str) -> AppResult<Vec<(String, i64)>> {
        let _ = source;
        nyi()
    }

    // --- Bericht ---------------------------------------------------------------------------

    async fn add_report_entries(&mut self, run_id: Uuid, entries: &[ReportEntry]) -> AppResult<()> {
        let _ = (run_id, entries);
        nyi()
    }

    /// Löscht den Bericht eines Laufs (vor einem frischen Lauf nach einem Dry-Run).
    async fn delete_report_entries(&mut self, run_id: Uuid) -> AppResult<()> {
        let _ = run_id;
        nyi()
    }

    async fn list_report_entries(
        &mut self,
        run_id: Uuid,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReportEntry>> {
        let _ = (run_id, severity, limit, offset);
        nyi()
    }

    /// Anzahl Berichtseinträge je Schweregrad.
    async fn report_counts(&mut self, run_id: Uuid) -> AppResult<Vec<(String, i64)>> {
        let _ = run_id;
        nyi()
    }

    // --- Zeilen schreiben ----------------------------------------------------------------

    /// `INSERT` einer Zeile; `organization_id` wird gesetzt, fehlende Spalten erhalten ihren Default.
    async fn insert_row(&mut self, table: ImportTable, row: &Value) -> AppResult<()> {
        let _ = (table, row);
        nyi()
    }

    /// Wie `insert_row` mit `ON CONFLICT DO NOTHING`; `true`, wenn eine Zeile entstand.
    async fn insert_row_ignore(&mut self, table: ImportTable, row: &Value) -> AppResult<bool> {
        let _ = (table, row);
        nyi()
    }

    /// Setzt die Spalten aus `row` für die Zeile `id`; erhöht `version`. `false`, wenn es sie nicht gibt.
    async fn update_row(&mut self, table: ImportTable, id: Uuid, row: &Value) -> AppResult<bool> {
        let _ = (table, id, row);
        nyi()
    }

    /// Löscht Zeilen, deren `column` (Whitelist-Name) gleich `id` ist (z. B. Label-Zuordnungen).
    async fn delete_rows_where(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
    ) -> AppResult<u64> {
        let _ = (table, column, id);
        nyi()
    }

    /// `updated_at` einer Zeile (für die Delta-Konflikterkennung).
    async fn row_updated_at(
        &mut self,
        table: ImportTable,
        id: Uuid,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let _ = (table, id);
        nyi()
    }

    // --- Lookups ---------------------------------------------------------------------------

    async fn find_user_id_by_email(&mut self, email: &str) -> AppResult<Option<Uuid>> {
        let _ = email;
        nyi()
    }

    async fn user_display_name_taken(&mut self, display_name: &str) -> AppResult<bool> {
        let _ = display_name;
        nyi()
    }

    /// Team mit diesem Key (aktuell oder früher, nicht im Papierkorb).
    async fn find_team_by_key(&mut self, key: &str) -> AppResult<Option<Uuid>> {
        let _ = key;
        nyi()
    }

    /// Alle aktuellen und früheren Team-Keys (für Kollisionsprüfung im Scan).
    async fn all_team_keys(&mut self) -> AppResult<Vec<String>> {
        nyi()
    }

    /// Nächste Nummer (`+1`, atomar) für Projekte (Scope = Lead-Team) oder Initiativen.
    async fn next_import_number(
        &mut self,
        counter: ImportCounter,
        scope: Option<Uuid>,
    ) -> AppResult<i32> {
        let _ = (counter, scope);
        nyi()
    }

    /// Phase 8: `team.last_issue_number`/`last_cycle_number`/`last_project_number` und
    /// `organization.last_initiative_number` auf mindestens das Maximum der vorhandenen Nummern setzen.
    async fn finalize_import_counters(&mut self, team_ids: &[Uuid]) -> AppResult<()> {
        let _ = team_ids;
        nyi()
    }

    /// Setzt `team.import_locked_until` (`None` hebt die Sperre auf).
    async fn set_team_import_lock(
        &mut self,
        team_ids: &[Uuid],
        until: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        let _ = (team_ids, until);
        nyi()
    }

    /// Standardstatus/Triage-/Auto-Close-Status eines Teams setzen (Zirkelreferenz Team/Status).
    async fn set_team_states(
        &mut self,
        team_id: Uuid,
        default_state: Option<Uuid>,
        triage_state: Option<Uuid>,
        auto_close_state: Option<Uuid>,
    ) -> AppResult<()> {
        let _ = (team_id, default_state, triage_state, auto_close_state);
        nyi()
    }

    /// Erster Status eines Teams der Typen (Reihenfolge) – Fallback für den Standardstatus.
    async fn first_state_of_types(
        &mut self,
        team_id: Uuid,
        types: &[&str],
    ) -> AppResult<Option<Uuid>> {
        let _ = (team_id, types);
        nyi()
    }

    /// Zeilen, deren Textspalte `needle` enthält (für das Umschreiben eingebetteter Dateien).
    /// Liefert `(id, Wert von owner_column bzw. id, Text)`.
    async fn find_rows_containing(
        &mut self,
        table: ImportTable,
        column: &str,
        owner_column: &str,
        needle: &str,
        after: Option<Uuid>,
        limit: i64,
    ) -> AppResult<Vec<(Uuid, Uuid, String)>> {
        let _ = (table, column, owner_column, needle, after, limit);
        nyi()
    }

    /// Setzt eine UUID-Spalte ohne `version`/`updated_at` zu ändern (Eltern-Verknüpfung).
    async fn set_row_uuid(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
        value: Option<Uuid>,
    ) -> AppResult<()> {
        let _ = (table, column, id, value);
        nyi()
    }

    /// Setzt eine Textspalte (ohne `version`/`updated_at` zu ändern).
    /// Würde `parent` als Eltern-Issue von `child` einen Zyklus bilden (`parent == child` oder
    /// `child` ist Vorfahr von `parent`)?
    async fn issue_parent_would_cycle(&mut self, child: Uuid, parent: Uuid) -> AppResult<bool> {
        let _ = (child, parent);
        nyi()
    }

    async fn set_row_text(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
        value: &str,
    ) -> AppResult<()> {
        let _ = (table, column, id, value);
        nyi()
    }

    /// Eine Zeile (Name ohne Beachtung der Groß-/Kleinschreibung, optional je Team) – für Statuswerte,
    /// die einem vorhandenen gleichnamigen Eintrag zugeordnet werden.
    async fn find_row_id_by_name(
        &mut self,
        table: ImportTable,
        name: &str,
        team_id: Option<Uuid>,
    ) -> AppResult<Option<Uuid>> {
        let _ = (table, name, team_id);
        nyi()
    }

    /// Status eines Teams mit Typ und Name (Name ohne Beachtung der Groß-/Kleinschreibung) –
    /// Zuordnung beim Import in ein bestehendes Team.
    async fn find_state_by_type_and_name(
        &mut self,
        team_id: Uuid,
        state_type: &str,
        name: &str,
    ) -> AppResult<Option<Uuid>> {
        let _ = (team_id, state_type, name);
        nyi()
    }

    /// Anzahl gemappter Objekte eines Typs, die in der Zieltabelle tatsächlich existieren (Verifikation).
    async fn count_mapped_rows(
        &mut self,
        source: &str,
        source_type: &str,
        table: ImportTable,
    ) -> AppResult<i64> {
        let _ = (source, source_type, table);
        nyi()
    }

    /// Anzahl der Zeilen einer Tabelle mit gegebener `import_mapping`-Typzuordnung (Verifikation).
    async fn count_rows(&mut self, table: ImportTable) -> AppResult<i64> {
        let _ = table;
        nyi()
    }
}
