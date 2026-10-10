//! Repository-Port `CommentRepo`: Kommentare, Reaktionen, Eltern-Auflösung (WP09).

use super::user::{OrderBy, PageCursor};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::*;
use uuid::Uuid;

/// Art des Elternobjekts eines Kommentars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParentKind {
    Issue,
    Project,
    Initiative,
    Document,
    Milestone,
    ProjectUpdate,
    InitiativeUpdate,
}

/// Elternobjekt eines Kommentars (genau eines, siehe Schema-Invariante).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParentRef {
    Issue(IssueId),
    Project(ProjectId),
    Initiative(InitiativeId),
    Document(DocumentId),
    Milestone(MilestoneId),
    ProjectUpdate(ProjectUpdateId),
    InitiativeUpdate(InitiativeUpdateId),
}

impl ParentRef {
    pub fn kind(&self) -> ParentKind {
        match self {
            Self::Issue(_) => ParentKind::Issue,
            Self::Project(_) => ParentKind::Project,
            Self::Initiative(_) => ParentKind::Initiative,
            Self::Document(_) => ParentKind::Document,
            Self::Milestone(_) => ParentKind::Milestone,
            Self::ProjectUpdate(_) => ParentKind::ProjectUpdate,
            Self::InitiativeUpdate(_) => ParentKind::InitiativeUpdate,
        }
    }

    pub fn uuid(&self) -> Uuid {
        match self {
            Self::Issue(i) => i.0,
            Self::Project(i) => i.0,
            Self::Initiative(i) => i.0,
            Self::Document(i) => i.0,
            Self::Milestone(i) => i.0,
            Self::ProjectUpdate(i) => i.0,
            Self::InitiativeUpdate(i) => i.0,
        }
    }

    pub fn from_parts(kind: ParentKind, id: Uuid) -> Self {
        match kind {
            ParentKind::Issue => Self::Issue(IssueId(id)),
            ParentKind::Project => Self::Project(ProjectId(id)),
            ParentKind::Initiative => Self::Initiative(InitiativeId(id)),
            ParentKind::Document => Self::Document(DocumentId(id)),
            ParentKind::Milestone => Self::Milestone(MilestoneId(id)),
            ParentKind::ProjectUpdate => Self::ProjectUpdate(ProjectUpdateId(id)),
            ParentKind::InitiativeUpdate => Self::InitiativeUpdate(InitiativeUpdateId(id)),
        }
    }
}

/// Wessen Sichtbarkeit über den Zugriff auf das Elternobjekt entscheidet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentScope {
    /// Alle diesen Teams (Issue: eines; Projekt: alle Teams des Projekts).
    Teams(Vec<TeamId>),
    /// Workspaceweit (Initiativen und ihre Updates).
    Workspace,
}

/// Ein Kommentar.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: CommentId,
    pub user_id: Option<UserId>,
    pub body: String,
    pub parent: ParentRef,
    /// Wurzelkommentar des Threads (`None` = Top-Level).
    pub parent_id: Option<CommentId>,
    pub quoted_text: Option<String>,
    pub edited_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolving_comment_id: Option<CommentId>,
    pub resolving_user_id: Option<UserId>,
    /// Gesetzt, wenn der Body durch den Platzhalter ersetzt wurde.
    pub deleted_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Neuer Kommentar.
#[derive(Debug, Clone, PartialEq)]
pub struct NewComment {
    pub id: CommentId,
    pub user_id: Option<UserId>,
    pub body: String,
    pub parent: ParentRef,
    pub parent_id: Option<CommentId>,
    pub quoted_text: Option<String>,
    pub now: DateTime<Utc>,
}

/// Abfrage einer Kommentarliste eines Elternobjekts (nicht archivierte).
#[derive(Debug, Clone)]
pub struct CommentQuery {
    pub parent: ParentRef,
    pub order_by: OrderBy,
    pub descending: bool,
    pub cursor: Option<PageCursor>,
    /// Zu ladende Zeilen (Seitengröße + 1).
    pub fetch: i64,
}

/// Ziel einer Reaktion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionTarget {
    Comment(CommentId),
    Issue(IssueId),
    ProjectUpdate(ProjectUpdateId),
    InitiativeUpdate(InitiativeUpdateId),
}

/// Eine Reaktion.
#[derive(Debug, Clone, PartialEq)]
pub struct Reaction {
    pub id: ReactionId,
    pub emoji: String,
    pub user_id: UserId,
    pub target: ReactionTarget,
    pub created_at: DateTime<Utc>,
}

/// Mögliche Erwähnung: aktiver User mit Rolle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionCandidate {
    pub id: UserId,
    pub display_name: String,
    pub role: UserRole,
}

fn not_implemented<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Persistenzzugriff des Bereichs `comment`.
#[async_trait]
#[allow(unused_variables)]
pub trait CommentRepo: Send {
    /// Löst die Referenz auf ein Elternobjekt auf (UUID, Kennung, Slug, Name). `None` = nicht gefunden;
    /// mehrdeutige Namen → `ambiguous_reference`.
    async fn find_parent(
        &mut self,
        kind: ParentKind,
        reference: &str,
    ) -> AppResult<Option<ParentRef>> {
        not_implemented()
    }
    /// Sichtbarkeitsbereich des Elternobjekts; `None`, wenn es nicht existiert (oder archiviert ist).
    async fn parent_scope(&mut self, parent: ParentRef) -> AppResult<Option<ParentScope>> {
        not_implemented()
    }
    /// Kennung `ENG-123` eines Issues.
    async fn issue_identifier(&mut self, issue: IssueId) -> AppResult<Option<String>> {
        not_implemented()
    }
    /// Erster Top-Level-Kommentar des Elternobjekts (Status-Updates haben genau einen Thread).
    async fn first_root(&mut self, parent: ParentRef) -> AppResult<Option<CommentId>> {
        not_implemented()
    }

    async fn insert(&mut self, c: &NewComment) -> AppResult<Comment> {
        not_implemented()
    }
    /// Auch archivierte; der Use-Case entscheidet.
    async fn get(&mut self, id: CommentId) -> AppResult<Option<Comment>> {
        not_implemented()
    }
    async fn list(&mut self, q: &CommentQuery) -> AppResult<Vec<Comment>> {
        not_implemented()
    }
    /// Nicht archivierte Antworten eines Wurzelkommentars.
    async fn count_replies(&mut self, root: CommentId) -> AppResult<i64> {
        not_implemented()
    }
    async fn update_body(
        &mut self,
        id: CommentId,
        body: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Comment> {
        not_implemented()
    }
    /// `placeholder = Some`: Body ersetzen und `deleted_at` setzen; `None`: archivieren.
    /// Reaktionen werden in beiden Fällen archiviert.
    async fn remove(
        &mut self,
        id: CommentId,
        placeholder: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        not_implemented()
    }
    /// Thread auflösen (`Some((user, resolving_comment))`) oder wieder öffnen (`None`).
    async fn set_resolution(
        &mut self,
        id: CommentId,
        resolution: Option<(UserId, Option<CommentId>)>,
        now: DateTime<Utc>,
    ) -> AppResult<Comment> {
        not_implemented()
    }

    /// Aktive User mit diesen Anzeigenamen (klein geschrieben).
    async fn mention_candidates(&mut self, names: &[String]) -> AppResult<Vec<MentionCandidate>> {
        not_implemented()
    }

    /// Reaktion anlegen; idempotent (`false` = existierte bereits).
    async fn add_reaction(
        &mut self,
        target: ReactionTarget,
        user: UserId,
        emoji: &str,
        now: DateTime<Utc>,
    ) -> AppResult<(Reaction, bool)> {
        not_implemented()
    }
    async fn get_reaction(&mut self, id: ReactionId) -> AppResult<Option<Reaction>> {
        not_implemented()
    }
    async fn delete_reaction(&mut self, id: ReactionId) -> AppResult<()> {
        not_implemented()
    }
    /// Nicht archivierte Reaktionen auf diese Kommentare.
    async fn reactions_of_comments(&mut self, ids: &[CommentId]) -> AppResult<Vec<Reaction>> {
        not_implemented()
    }

    /// Abonniert den User (Issue bzw. Thread); `true`, wenn neu angelegt.
    async fn subscribe(
        &mut self,
        issue: IssueId,
        user: UserId,
        thread: Option<CommentId>,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        not_implemented()
    }
}
