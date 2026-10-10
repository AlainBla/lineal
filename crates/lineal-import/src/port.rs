//! Port `ImportSource` (docs/13 §2): Lesezugriff auf ein Fremdsystem.
//!
//! Alle Adapter liefern Objekte im **normalisierten Linear-Format**: JSON-Objekte mit den
//! camelCase-Feldern der Linear-GraphQL-API, Verbindungen (`{ "nodes": [...] }`) bereits zu
//! Arrays aufgelöst, Referenzen als `{ "id": "…" }`. Die Engine kennt nur dieses Format.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fmt;
use std::time::Duration;

/// Entitätstypen, die eine Quelle seitenweise liefert. `as_str` ist zugleich der `source_type`
/// in `import_mapping`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceEntity {
    Users,
    Teams,
    TeamMemberships,
    WorkflowStates,
    IssueLabels,
    ProjectStatuses,
    ProjectLabels,
    Templates,
    Cycles,
    Initiatives,
    Projects,
    ProjectMilestones,
    Issues,
    IssueRelations,
    IssueHistory,
    IssueSubscriptions,
    Comments,
    Attachments,
    Documents,
    ProjectUpdates,
    InitiativeUpdates,
    CustomViews,
    Favorites,
}

impl SourceEntity {
    /// Alle Entitäten in Scan-Reihenfolge.
    pub const ALL: [SourceEntity; 23] = [
        Self::Users,
        Self::Teams,
        Self::TeamMemberships,
        Self::WorkflowStates,
        Self::IssueLabels,
        Self::ProjectStatuses,
        Self::ProjectLabels,
        Self::Templates,
        Self::Cycles,
        Self::Initiatives,
        Self::Projects,
        Self::ProjectMilestones,
        Self::Issues,
        Self::IssueRelations,
        Self::IssueHistory,
        Self::IssueSubscriptions,
        Self::Comments,
        Self::Attachments,
        Self::Documents,
        Self::ProjectUpdates,
        Self::InitiativeUpdates,
        Self::CustomViews,
        Self::Favorites,
    ];

    /// `source_type` in `import_mapping` und Schlüssel in Statistik/Scan.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Users => "User",
            Self::Teams => "Team",
            Self::TeamMemberships => "TeamMembership",
            Self::WorkflowStates => "WorkflowState",
            Self::IssueLabels => "IssueLabel",
            Self::ProjectStatuses => "ProjectStatus",
            Self::ProjectLabels => "ProjectLabel",
            Self::Templates => "Template",
            Self::Cycles => "Cycle",
            Self::Initiatives => "Initiative",
            Self::Projects => "Project",
            Self::ProjectMilestones => "ProjectMilestone",
            Self::Issues => "Issue",
            Self::IssueRelations => "IssueRelation",
            Self::IssueHistory => "IssueHistory",
            Self::IssueSubscriptions => "IssueSubscription",
            Self::Comments => "Comment",
            Self::Attachments => "Attachment",
            Self::Documents => "Document",
            Self::ProjectUpdates => "ProjectUpdate",
            Self::InitiativeUpdates => "InitiativeUpdate",
            Self::CustomViews => "CustomView",
            Self::Favorites => "Favorite",
        }
    }
}

/// Eine Seitenanfrage.
#[derive(Debug, Clone)]
pub struct PageQuery {
    pub entity: SourceEntity,
    /// Undurchsichtiger Cursor der Quelle (aus `SourcePage::next_cursor`).
    pub cursor: Option<String>,
    /// Nur Objekte, die seit diesem Zeitpunkt geändert wurden (Delta-Import).
    pub updated_after: Option<DateTime<Utc>>,
    /// Seitengröße (50–100).
    pub limit: u32,
}

/// Eine Seite normalisierter Objekte.
#[derive(Debug, Clone, Default)]
pub struct SourcePage {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
}

/// Der Workspace der Quelle.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceWorkspace {
    pub id: String,
    pub name: String,
    pub url_key: String,
    /// E-Mail des Token-Inhabers (zur Anzeige im Plan).
    pub viewer_email: Option<String>,
}

/// Heruntergeladene Datei (bereits geprüft: Größe, Content-Type, Host).
#[derive(Debug, Clone)]
pub struct Downloaded {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Fehler einer Quelle. Meldungen enthalten **nie** das Token oder Antwortinhalte.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceError {
    /// Token ungültig oder ohne Berechtigung (HTTP 401/403).
    Auth,
    /// Rate-Limit; optional die Wartezeit bis zum Reset.
    RateLimited(Option<Duration>),
    /// Netzwerk-/Serverfehler (wiederholbar).
    Transport(String),
    /// Ungültige Antwort oder Anfrage (nicht wiederholbar).
    Invalid(String),
    /// Download abgelehnt (Host, IP, Größe, Content-Type).
    Rejected(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth => f.write_str("the source rejected the credentials"),
            Self::RateLimited(_) => f.write_str("the source rate limit was exceeded"),
            Self::Transport(m) => write!(f, "source request failed: {m}"),
            Self::Invalid(m) => write!(f, "invalid response from the source: {m}"),
            Self::Rejected(m) => write!(f, "download rejected: {m}"),
        }
    }
}

impl std::error::Error for SourceError {}

pub type SourceResult<T> = Result<T, SourceError>;

/// Quelle für den Import. Ausschließlich lesend.
#[async_trait]
pub trait ImportSource: Send + Sync {
    /// `linear` oder `linearCsv` (Wert in `import_run.source`).
    fn kind(&self) -> &'static str;

    /// Hinweise zur Quelle (z. B. fehlende Daten beim CSV-Import) für den Bericht.
    fn notices(&self) -> Vec<String> {
        Vec::new()
    }

    /// Verbindung prüfen und Workspace lesen.
    async fn workspace(&self) -> SourceResult<SourceWorkspace>;

    /// Eine Seite der Entität lesen.
    async fn page(&self, query: &PageQuery) -> SourceResult<SourcePage>;

    /// Datei herunterladen (nur https-Hosts der Quelle, keine privaten IPs, Größen-/Typlimit).
    async fn download(&self, url: &str) -> SourceResult<Downloaded>;

    /// Alle Seiten einer Entität lesen (Hilfsfunktion für kleine Mengen und den Scan).
    async fn collect(
        &self,
        entity: SourceEntity,
        updated_after: Option<DateTime<Utc>>,
        max_pages: usize,
    ) -> SourceResult<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor = None;
        for _ in 0..max_pages {
            let page = self
                .page(&PageQuery {
                    entity,
                    cursor: cursor.clone(),
                    updated_after,
                    limit: 100,
                })
                .await?;
            out.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(out),
            }
        }
        Ok(out)
    }
}
