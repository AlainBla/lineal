//! Repository-Port `DocumentRepo`: Dokumente, `DocumentContent`, Snapshot-Historie (WP10, docs/06 §1).
//!
//! Der kanonische Inhalt liegt in `document_content` (mit `version`); `document.content` spiegelt
//! ihn, damit andere Lesepfade (Suche, Ressourcenlisten) ohne Join auskommen. Beides schreibt
//! ausschließlich [`DocumentRepo::set_content`] (und `insert`).

use super::user::{OrderBy, PageCursor};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::*;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Das eine Elternobjekt eines Dokuments (docs/06 §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocParent {
    Project(ProjectId),
    Issue(IssueId),
    Initiative(InitiativeId),
    /// Cycle-Dokument; `team` ist das Team des Cycles (Zugriffsspalte laut Schema).
    Cycle {
        cycle: CycleId,
        team: TeamId,
    },
    Team(TeamId),
}

impl DocParent {
    /// Name der Elternart (für Fehlermeldungen und Ausgaben).
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Project(_) => "project",
            Self::Issue(_) => "issue",
            Self::Initiative(_) => "initiative",
            Self::Cycle { .. } => "cycle",
            Self::Team(_) => "team",
        }
    }
}

/// Ein Dokument samt aktuellem Inhalt.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: DocumentId,
    pub slug_id: String,
    pub title: String,
    /// Aktueller Markdown-Inhalt (aus `document_content`).
    pub content: String,
    pub content_id: Uuid,
    /// Revision des Inhalts (`document_content.version`).
    pub content_version: i32,
    pub restored_at: Option<DateTime<Utc>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub project_id: Option<ProjectId>,
    pub issue_id: Option<IssueId>,
    pub initiative_id: Option<InitiativeId>,
    pub cycle_id: Option<CycleId>,
    pub team_id: Option<TeamId>,
    pub creator_id: Option<UserId>,
    pub updated_by_id: Option<UserId>,
    pub sort_order: f64,
    pub last_applied_template_id: Option<TemplateId>,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub trashed_with_parent: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl Document {
    /// Das Elternobjekt (genau eines laut Schema-Check).
    pub fn parent(&self) -> Option<DocParent> {
        if let Some(p) = self.project_id {
            Some(DocParent::Project(p))
        } else if let Some(i) = self.issue_id {
            Some(DocParent::Issue(i))
        } else if let Some(i) = self.initiative_id {
            Some(DocParent::Initiative(i))
        } else if let (Some(c), Some(t)) = (self.cycle_id, self.team_id) {
            Some(DocParent::Cycle { cycle: c, team: t })
        } else {
            self.team_id.map(DocParent::Team)
        }
    }

    /// Setzt das Elternobjekt (alle anderen Fremdschlüssel werden geleert).
    pub fn set_parent(&mut self, parent: DocParent) {
        self.project_id = None;
        self.issue_id = None;
        self.initiative_id = None;
        self.cycle_id = None;
        self.team_id = None;
        match parent {
            DocParent::Project(p) => self.project_id = Some(p),
            DocParent::Issue(i) => self.issue_id = Some(i),
            DocParent::Initiative(i) => self.initiative_id = Some(i),
            DocParent::Cycle { cycle, team } => {
                self.cycle_id = Some(cycle);
                self.team_id = Some(team);
            }
            DocParent::Team(t) => self.team_id = Some(t),
        }
    }
}

/// Neues Dokument inklusive erstem Inhalt.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDocument {
    pub id: DocumentId,
    pub content_id: Uuid,
    pub title: String,
    pub markdown: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub parent: DocParent,
    pub creator_id: Option<UserId>,
    pub sort_order: f64,
    pub last_applied_template_id: Option<TemplateId>,
    pub now: DateTime<Utc>,
}

/// Zustand des Elternobjekts (Schreibschutz bei Archiv/Papierkorb, docs/06 §1.4 Regel 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentState {
    pub archived: bool,
    pub trashed: bool,
}

/// Wem ein Dokument über sein Elternobjekt sichtbar ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentScope {
    /// Teams des Elternobjekts (Issue: eines, Projekt: alle, Cycle/Team: eines, Initiative: Lead-Team).
    pub teams: Vec<TeamId>,
    /// Initiative: workspaceweit sichtbar (außer für Gäste), `teams` enthält dann nur das Lead-Team.
    pub workspace: bool,
    pub state: ParentState,
}

/// Filter und Sichtbarkeit für [`DocumentRepo::list`].
#[derive(Debug, Clone, Default)]
pub struct DocumentQuery {
    /// Teams, die der Akteur sehen darf (`policy::visible_team_ids`).
    pub visible_teams: Vec<TeamId>,
    /// Akteur (für geteilte Issues); `None` bei System.
    pub actor: Option<UserId>,
    /// Gäste sehen keine Initiativen.
    pub initiatives_visible: bool,
    /// Volltext über Titel und Inhalt (Teilwort, case-insensitiv).
    pub text: Option<String>,
    pub creator: Option<UserId>,
    pub project: Option<ProjectId>,
    pub initiative: Option<InitiativeId>,
    pub issue: Option<IssueId>,
    pub cycle: Option<CycleId>,
    /// Elternobjekt ist das Team, ein Cycle/Issue des Teams oder ein Projekt mit diesem Team.
    pub team: Option<TeamId>,
    /// Papierkorb statt Normalansicht.
    pub trashed: bool,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub order_by: OrderBy,
    pub cursor: Option<PageCursor>,
    /// Anzahl geladener Zeilen (Aufrufer: `limit + 1`).
    pub fetch: i64,
}

/// Art eines Snapshots (`document_content_history.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Created,
    Edited,
    Restored,
}

impl HistoryKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Edited => "edited",
            Self::Restored => "restored",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(Self::Created),
            "edited" => Some(Self::Edited),
            "restored" => Some(Self::Restored),
            _ => None,
        }
    }
}

/// Ein Snapshot der Historie.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub document_content_id: Uuid,
    pub markdown: String,
    pub title: Option<String>,
    pub actor_ids: Vec<UserId>,
    pub content_snapshot_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub kind: HistoryKind,
    pub restored_from_id: Option<Uuid>,
}

/// Neuer Snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct NewHistory {
    pub id: Uuid,
    pub document_content_id: Uuid,
    pub markdown: String,
    pub title: Option<String>,
    pub actor_ids: Vec<UserId>,
    pub kind: HistoryKind,
    pub restored_from_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

/// Persistenzzugriff des Bereichs `document`. Alle Abfragen laufen im Workspace der Transaktion.
#[async_trait]
pub trait DocumentRepo: Send {
    /// Legt Dokument und `DocumentContent` (Version 1) an. `slug_id` vergibt die Datenbank.
    async fn insert(&mut self, d: &NewDocument) -> AppResult<Document> {
        let _ = d;
        unimplemented_repo()
    }

    /// Per UUID, auch im Papierkorb (der Aufrufer prüft `trashed`).
    async fn get(&mut self, id: DocumentId) -> AppResult<Option<Document>> {
        let _ = id;
        unimplemented_repo()
    }

    /// Per `slugId` (12 Hex-Zeichen), auch im Papierkorb.
    async fn find_by_slug_id(&mut self, slug_id: &str) -> AppResult<Option<Document>> {
        let _ = slug_id;
        unimplemented_repo()
    }

    /// Schreibt Titel, Icon, Farbe, Elternobjekt, Sortierung, Papierkorb- und Template-Felder;
    /// erhöht `version`, setzt `updated_at = now` und `updated_by_id`.
    async fn update(
        &mut self,
        d: &Document,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Document> {
        let _ = (d, updated_by, now);
        unimplemented_repo()
    }

    /// Ersetzt den Inhalt (`document_content` und Spiegel in `document.content`), erhöht die
    /// Inhalts-Revision und liefert sie zurück. `restored` setzt `restored_at`.
    async fn set_content(
        &mut self,
        id: DocumentId,
        markdown: &str,
        restored: bool,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<i32> {
        let _ = (id, markdown, restored, updated_by, now);
        unimplemented_repo()
    }

    /// Größter `sort_order` im Elternobjekt (nicht im Papierkorb).
    async fn max_sort_order(&mut self, parent: DocParent) -> AppResult<Option<f64>> {
        let _ = parent;
        unimplemented_repo()
    }

    /// Dokumente nach Filter und Sichtbarkeit (sortiert absteigend nach `order_by`, dann `id`).
    async fn list(&mut self, q: &DocumentQuery) -> AppResult<Vec<Document>> {
        let _ = q;
        unimplemented_repo()
    }

    /// Sichtbarkeitsbereich und Zustand des Elternobjekts; `None`, wenn es nicht existiert.
    async fn parent_scope(&mut self, parent: DocParent) -> AppResult<Option<ParentScope>> {
        let _ = parent;
        unimplemented_repo()
    }

    /// Ob das Issue mit dem User geteilt wurde (`issue_share`).
    async fn issue_shared_with(&mut self, issue: IssueId, user: UserId) -> AppResult<bool> {
        let _ = (issue, user);
        unimplemented_repo()
    }

    /// Abo anlegen (idempotent).
    async fn subscribe(
        &mut self,
        id: Uuid,
        document: DocumentId,
        user: UserId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let _ = (id, document, user, reason, now);
        unimplemented_repo()
    }

    /// Abonnenten (aktive Abos).
    async fn subscribers(&mut self, document: DocumentId) -> AppResult<Vec<UserId>> {
        let _ = document;
        unimplemented_repo()
    }

    /// Neuer Snapshot in der Historie (append-only).
    async fn history_insert(&mut self, h: &NewHistory) -> AppResult<HistoryEntry> {
        let _ = h;
        unimplemented_repo()
    }

    /// Jüngster Snapshot eines Inhalts.
    async fn history_latest(&mut self, content_id: Uuid) -> AppResult<Option<HistoryEntry>> {
        let _ = content_id;
        unimplemented_repo()
    }

    /// Historie, neueste zuerst (`before` = Keyset über `content_snapshot_at`, `id`).
    async fn history_list(
        &mut self,
        content_id: Uuid,
        before: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> AppResult<Vec<HistoryEntry>> {
        let _ = (content_id, before, limit);
        unimplemented_repo()
    }

    /// Ein Snapshot, nur wenn er zum Inhalt gehört.
    async fn history_get(&mut self, content_id: Uuid, id: Uuid) -> AppResult<Option<HistoryEntry>> {
        let _ = (content_id, id);
        unimplemented_repo()
    }

    /// Legt alle Dokumente des Elternobjekts in den Papierkorb (`trashed_with_parent`); liefert ihre IDs.
    async fn cascade_trash(
        &mut self,
        parent: DocParent,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DocumentId>> {
        let _ = (parent, now);
        unimplemented_repo()
    }

    /// Stellt genau die mit dem Elternobjekt mitgelöschten Dokumente wieder her.
    async fn cascade_restore(&mut self, parent: DocParent) -> AppResult<Vec<DocumentId>> {
        let _ = parent;
        unimplemented_repo()
    }
}
