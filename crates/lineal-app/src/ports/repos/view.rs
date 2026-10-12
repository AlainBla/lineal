//! Repository-Port `ViewRepo`: Custom Views, Anzeigeoptionen, Favoriten, Suche (WP11,
//! docs/05 §2–5).
//!
//! `FilterEval` ist der Auswertungskontext des Filter-Compilers (`lineal-db/src/filter_sql.rs`):
//! Er trägt alles, was der reine Filterbaum zur Laufzeit braucht (Zeitpunkt, Zeitzone, „ich“,
//! sichtbare Teams, Team-/Initiativ-Hierarchie). Die Sichtbarkeitsmenge steht **außerhalb** des
//! Benutzerfilters und wird vom Compiler immer per UND angehängt.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank
//! kompilieren.

use super::user::{Page, PageRequest};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::{InitiativeId, ProjectId, TeamId, UserId, ViewId};
use lineal_domain::rules::filter::{InitiativeFilter, IssueFilter, ProjectFilter};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Auswertungskontext eines Filters (docs/05 §1.10 `ctx`).
#[derive(Debug, Clone, PartialEq)]
pub struct FilterEval {
    /// Auswertungszeitpunkt (relative Dauern, aktive Cycles, SLA-Status).
    pub now: DateTime<Utc>,
    /// IANA-Zeitzone des Auswertenden (`User.timezone`), ersatzweise `UTC`.
    pub timezone: String,
    /// Workspace (jede Teilabfrage trägt `organization_id`).
    pub org: lineal_domain::ids::OrgId,
    /// Gäste sehen keine Initiativen: Filter über Initiativen treffen für sie nichts.
    pub is_guest: bool,
    /// Aufrufender User (`isMe`); `None` bei System/App ohne User → `isMe` trifft nichts.
    pub me: Option<UserId>,
    /// `visibleTeamIds` des Aufrufers (Berechtigungsfilter, immer UND).
    pub visible_teams: Vec<TeamId>,
    /// Team → alle *sichtbaren* Nachfahren (für `team.ancestors.some.id`).
    pub descendants: BTreeMap<Uuid, Vec<Uuid>>,
    /// Kanten `(parent, child)` des Initiative-DAG (für „Initiative X inkl. Sub-Initiativen“).
    pub initiative_edges: Vec<(Uuid, Uuid)>,
    /// `true`: Referenzen (Teams, User, Status, Labels …) müssen existieren und sichtbar sein,
    /// sonst `not_found` (Ad-hoc-Filter, Speichern einer View). `false` (gespeicherte View):
    /// unauflösbare Referenzen treffen einfach nichts (docs/05 §1.8 Punkt 5).
    pub strict_refs: bool,
}

/// Entität, die eine Custom View anzeigt (`modelName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewModel {
    Issue,
    Project,
    Initiative,
}

impl ViewModel {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Issue => "Issue",
            Self::Project => "Project",
            Self::Initiative => "Initiative",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Issue" => Some(Self::Issue),
            "Project" => Some(Self::Project),
            "Initiative" => Some(Self::Initiative),
            _ => None,
        }
    }
}

/// Eine gespeicherte Ansicht (Tabelle `custom_view`).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomView {
    pub id: ViewId,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub slug_id: String,
    pub model: ViewModel,
    /// Normalisierter Filter (je nach `model` aus `filter_data`/`project_filter_data`/
    /// `initiative_filter_data`).
    pub filter: Value,
    pub owner_id: UserId,
    pub creator_id: UserId,
    pub updated_by_id: Option<UserId>,
    pub shared: bool,
    pub team_id: Option<TeamId>,
    pub project_id: Option<ProjectId>,
    pub initiative_id: Option<InitiativeId>,
    pub sort_order: f64,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Neue Ansicht (Filter ist bereits validiert und normalisiert).
#[derive(Debug, Clone, PartialEq)]
pub struct NewCustomView {
    pub id: ViewId,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub model: ViewModel,
    pub filter: Value,
    pub owner_id: UserId,
    pub creator_id: UserId,
    pub shared: bool,
    pub team_id: Option<TeamId>,
    pub project_id: Option<ProjectId>,
    pub initiative_id: Option<InitiativeId>,
    pub sort_order: f64,
    pub now: DateTime<Utc>,
}

/// Wer eine Ansicht sehen darf (docs/05 §4.2). Wird vom Use-Case aus Policy und Mitgliedschaft
/// gebaut; das Repository wendet sie in jeder Abfrage an – unsichtbare Views existieren nicht.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewAccess {
    pub user_id: UserId,
    pub is_guest: bool,
    /// `visibleTeamIds` des Akteurs (bei Gästen nur ihre Teams).
    pub visible_teams: Vec<TeamId>,
}

/// Filter der Ansichtsliste (`list_custom_views`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewListFilter {
    pub model: Option<ViewModel>,
    pub team: Option<TeamId>,
    pub shared: Option<bool>,
    /// Teilstring im Namen (case-insensitiv).
    pub query: Option<String>,
    /// `false`: an Projekte/Initiativen gebundene Views weglassen (`list_custom_views`).
    pub include_scoped: bool,
}

/// Schlüssel einer Ansicht für Anzeigeoptionen (docs/05 §2.2): Custom View oder vordefiniert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewKey {
    pub custom_view_id: Option<ViewId>,
    /// Kanonischer `viewType` (docs/05 §3.5), z. B. `teamActive`.
    pub view_type: Option<String>,
    /// Team/Projekt hinter einer vordefinierten Ansicht.
    pub view_target_id: Option<Uuid>,
}

/// Ebene einer `ViewPreferences`-Zeile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefLevel {
    Organization,
    User,
}

impl PrefLevel {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::User => "user",
        }
    }
}

/// Ein Favorit (docs/05 §4.6); genau ein Zielfeld ist gesetzt (bei `folder`: keines).
#[derive(Debug, Clone, PartialEq)]
pub struct Favorite {
    pub id: Uuid,
    pub owner_id: UserId,
    /// `folder`, `issue`, `project`, `cycle`, `customView`, `predefinedView`, `document`,
    /// `initiative`, `label`, `projectLabel`, `team`, `user`.
    pub kind: String,
    pub parent_id: Option<Uuid>,
    pub folder_name: Option<String>,
    pub sort_order: f64,
    /// ID des Ziels (je nach `kind` die passende Spalte).
    pub target_id: Option<Uuid>,
    pub predefined_view_type: Option<String>,
    pub predefined_view_team_id: Option<TeamId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Favorit.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFavorite {
    pub id: Uuid,
    pub owner_id: UserId,
    pub kind: String,
    pub parent_id: Option<Uuid>,
    pub folder_name: Option<String>,
    pub sort_order: f64,
    pub target_id: Option<Uuid>,
    pub predefined_view_type: Option<String>,
    pub predefined_view_team_id: Option<TeamId>,
    pub now: DateTime<Utc>,
}

/// Abo eines Users auf eine Custom View (Zeile `notification_subscription` mit
/// `target_type = 'customView'`, docs/05 §4.7).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSubscription {
    pub id: Uuid,
    pub subscriber_id: UserId,
    pub view_id: ViewId,
    /// `issueAddedToView`, `issueStatusChanged`, `issueStatusChangedAll`.
    pub types: Vec<String>,
    pub active: bool,
}

/// Filterbaum einer Art, wie ihn `verify_filter_refs` prüft.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyFilter {
    Issue(IssueFilter),
    Project(ProjectFilter),
    Initiative(InitiativeFilter),
}

/// Art eines Suchtreffers (docs/05 §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    Issue,
    Project,
    Initiative,
    Document,
}

/// Sortierung der Suche (Default Relevanz, docs/05 §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchOrder {
    #[default]
    Relevance,
    UpdatedAt,
    CreatedAt,
}

/// Suchanfrage. Die Berechtigung (`access`) wird in jeder Teilabfrage als UND angehängt.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    pub term: String,
    pub kinds: Vec<SearchKind>,
    pub access: ViewAccess,
    /// Einschränkung auf diese Teams (Issues und Projekte); bereits auf sichtbare Teams geschnitten.
    pub team_scope: Option<Vec<TeamId>>,
    pub include_comments: bool,
    pub include_archived: bool,
    pub order: SearchOrder,
    /// Zusatzfilter für Issues samt Kontext (UND mit der Suche).
    pub ast: Option<Box<IssueFilter>>,
    pub eval: Option<FilterEval>,
    /// Höchstens so viele Treffer (Offset + Limit, höchstens 500).
    pub limit: i64,
}

/// Ein Suchtreffer (nur Objekte, die der Aufrufer sehen darf).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHitRow {
    pub kind: SearchKind,
    pub id: Uuid,
    /// `ENG-124`, `P-ENG-7`, `I-4`; bei Dokumenten `None`.
    pub identifier: Option<String>,
    pub title: String,
    pub team_key: Option<String>,
    /// Statuskategorie (Issue), Status (Projekt/Initiative).
    pub status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived: bool,
    /// Exakter Kennungstreffer (steht an erster Stelle).
    pub exact: bool,
    pub rank: f64,
    pub boost: i32,
    /// `(Feld, Ausschnitt)`; Fundstellen mit `<mark>…</mark>`, übriger Text HTML-escaped.
    pub highlights: Vec<(String, String)>,
}

/// Persistenzzugriff des Bereichs `view`.
#[async_trait]
pub trait ViewRepo: Send {
    /// Volltextsuche über Issues, Projekte, Initiativen und Dokumente (nur sichtbare Objekte).
    async fn search(&mut self, _q: &SearchQuery) -> AppResult<Vec<SearchHitRow>> {
        unimplemented_repo()
    }

    // --- Custom Views ---
    async fn insert_view(&mut self, _v: &NewCustomView) -> AppResult<CustomView> {
        unimplemented_repo()
    }
    /// Sichtbare Ansicht per UUID (auch archivierte, damit Wiederherstellen/Löschen möglich ist).
    async fn get_view(
        &mut self,
        _access: &ViewAccess,
        _id: ViewId,
    ) -> AppResult<Option<CustomView>> {
        unimplemented_repo()
    }
    /// Sichtbare, nicht archivierte Ansichten zu einer Referenz: UUID, `slugId`, dann exakter
    /// Name (case-insensitiv). Mehrere Treffer = mehrdeutig.
    async fn find_views(
        &mut self,
        _access: &ViewAccess,
        _reference: &str,
    ) -> AppResult<Vec<CustomView>> {
        unimplemented_repo()
    }
    async fn list_views(
        &mut self,
        _access: &ViewAccess,
        _filter: &ViewListFilter,
        _page: &PageRequest,
    ) -> AppResult<Page<CustomView>> {
        unimplemented_repo()
    }
    /// Schreibt alle veränderlichen Felder zurück (`version + 1`, `updated_at = now`).
    async fn save_view(
        &mut self,
        _v: &CustomView,
        _updated_by: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<CustomView> {
        unimplemented_repo()
    }
    /// Löscht die Ansicht endgültig samt Preferences, Favoriten und Abos (per FK).
    async fn delete_view(&mut self, _id: ViewId) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Prüft, dass alle im Filter genannten Referenzen existieren und für `visible_teams`
    /// sichtbar sind (`not_found` mit `details.reference`, docs/05 §1.8 Punkt 2).
    async fn verify_filter_refs(
        &mut self,
        _filter: &AnyFilter,
        _eval: &FilterEval,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    // --- Anzeigeoptionen ---
    /// `(organization, user)`-Zeilen der Ansicht.
    async fn get_preferences(
        &mut self,
        _key: &ViewKey,
        _user: UserId,
    ) -> AppResult<(Option<Value>, Option<Value>)> {
        unimplemented_repo()
    }
    async fn put_preferences(
        &mut self,
        _key: &ViewKey,
        _level: PrefLevel,
        _user: Option<UserId>,
        _prefs: &Value,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn delete_preferences(
        &mut self,
        _key: &ViewKey,
        _level: PrefLevel,
        _user: Option<UserId>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }

    // --- Favoriten ---
    async fn list_favorites(&mut self, _owner: UserId) -> AppResult<Vec<Favorite>> {
        unimplemented_repo()
    }
    async fn get_favorite(&mut self, _owner: UserId, _id: Uuid) -> AppResult<Option<Favorite>> {
        unimplemented_repo()
    }
    async fn insert_favorite(&mut self, _f: &NewFavorite) -> AppResult<Favorite> {
        unimplemented_repo()
    }
    /// Löscht einen Favoriten; Kinder eines Ordners rücken auf die oberste Ebene.
    async fn delete_favorite(&mut self, _owner: UserId, _id: Uuid) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Größter `sort_order` der obersten Ebene bzw. des Ordners.
    async fn max_favorite_sort_order(
        &mut self,
        _owner: UserId,
        _parent: Option<Uuid>,
    ) -> AppResult<Option<f64>> {
        unimplemented_repo()
    }
    /// Existiert das Ziel und ist es für den Akteur sichtbar? (`kind` wie in [`Favorite`].)
    async fn favorite_target_visible(
        &mut self,
        _kind: &str,
        _target: Uuid,
        _access: &ViewAccess,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Setzt Position und Ordner eines eigenen Favoriten (`None` = oberste Ebene).
    async fn update_favorite(
        &mut self,
        _owner: UserId,
        _id: Uuid,
        _parent: Option<Uuid>,
        _sort_order: f64,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<Favorite>> {
        unimplemented_repo()
    }

    // --- View-Abos (WP31; Auslösen der Benachrichtigung: WP32, docs/dev/schnittstellen.md) ---
    /// Legt das Abo des Users auf die View an oder ändert es (je Paar höchstens eines).
    async fn upsert_view_subscription(
        &mut self,
        _user: UserId,
        _view: ViewId,
        _types: &[String],
        _active: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<ViewSubscription> {
        unimplemented_repo()
    }
    async fn get_view_subscription(
        &mut self,
        _user: UserId,
        _view: ViewId,
    ) -> AppResult<Option<ViewSubscription>> {
        unimplemented_repo()
    }
    /// Alle aktiven Abos auf die View, die den Ereignistyp `ntype` enthalten (Fan-out-Hook).
    async fn active_view_subscriptions(
        &mut self,
        _view: ViewId,
        _ntype: &str,
    ) -> AppResult<Vec<ViewSubscription>> {
        unimplemented_repo()
    }
    /// Alle aktiven Abos des Workspace, die den Ereignistyp `ntype` enthalten (Einstieg des
    /// Fan-outs: je Abo `get_view` mit den Rechten des Abonnenten, dann Filter prüfen).
    async fn subscribed_views(&mut self, _ntype: &str) -> AppResult<Vec<ViewSubscription>> {
        unimplemented_repo()
    }
    /// Deaktiviert alle Abos außer dem des genannten Users (View verliert Sichtbarkeit).
    async fn deactivate_view_subscriptions_except(
        &mut self,
        _view: ViewId,
        _keep: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }
}
