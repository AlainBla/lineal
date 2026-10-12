//! Repository-Port `NotificationRepo` (WP08): Notifications, Abos (Issue, Objekt), Einstellungen
//! sowie die Lesezugriffe, die der Fan-out und die periodischen Jobs brauchen (die Jobs kennen
//! nur Ports, kein SQL).

use crate::error::{AppError, AppResult};
use crate::ports::repos::user::{Page, PageRequest};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{CommentId, IssueId, NotificationId, ProjectId, TeamId, UserId};
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Eine gespeicherte Benachrichtigung.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub user_id: UserId,
    /// `Notification.type`, z. B. `issueNewComment`.
    pub kind: String,
    pub category: String,
    /// `issue`, `project`, `initiative`, `document`, `system`.
    pub target_type: String,
    pub actor_id: Option<UserId>,
    pub issue_id: Option<IssueId>,
    pub team_id: Option<TeamId>,
    pub project_id: Option<ProjectId>,
    pub project_update_id: Option<Uuid>,
    pub project_milestone_id: Option<Uuid>,
    pub initiative_id: Option<Uuid>,
    pub initiative_update_id: Option<Uuid>,
    pub document_id: Option<Uuid>,
    pub comment_id: Option<CommentId>,
    pub parent_comment_id: Option<CommentId>,
    pub reaction_emoji: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub snoozed_until_at: Option<DateTime<Utc>>,
    pub unsnoozed_at: Option<DateTime<Utc>>,
    pub emailed_at: Option<DateTime<Utc>>,
    pub is_priority: bool,
    /// `issue:<id>`, `project:<id>`, `initiative:<id>`, `document:<id>` oder `notification:<id>`.
    pub group_key: String,
    /// Typen aller zusammengefassten Ereignisse (docs/05 §6.6, 1-Minuten-Bündelung); leer, wenn
    /// die Notification nicht gebündelt wurde. `kind` ist der spezifischste davon.
    pub bundled_types: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Notification mit den Anzeige-Daten des Bezugsobjekts (Joins).
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationView {
    pub n: Notification,
    pub actor_name: Option<String>,
    pub issue_identifier: Option<String>,
    /// Aktuelles Team des Issues (für Sichtbarkeitsprüfungen zum Lesezeitpunkt).
    pub issue_team_id: Option<TeamId>,
    pub issue_title: Option<String>,
    pub project_name: Option<String>,
    pub initiative_name: Option<String>,
    pub document_title: Option<String>,
    /// Empfänger (für die E-Mail).
    pub recipient_email: String,
}

/// Neue Benachrichtigung; `dedup_key` macht das Schreiben idempotent.
#[derive(Debug, Clone, PartialEq)]
pub struct NewNotification {
    pub id: NotificationId,
    pub user_id: UserId,
    pub kind: String,
    pub category: String,
    pub target_type: String,
    pub actor_id: Option<UserId>,
    pub issue_id: Option<IssueId>,
    pub team_id: Option<TeamId>,
    pub project_id: Option<ProjectId>,
    pub project_update_id: Option<Uuid>,
    pub project_milestone_id: Option<Uuid>,
    pub initiative_id: Option<Uuid>,
    pub initiative_update_id: Option<Uuid>,
    pub document_id: Option<Uuid>,
    pub comment_id: Option<CommentId>,
    pub parent_comment_id: Option<CommentId>,
    pub reaction_emoji: Option<String>,
    pub is_priority: bool,
    pub dedup_key: String,
    pub now: DateTime<Utc>,
}

/// Abfrage der Inbox (einzelne Notifications, neueste zuerst).
#[derive(Debug, Clone, Default)]
pub struct InboxQuery {
    pub unread_only: bool,
    /// Gesnoozte (noch nicht wieder vorgelegte) einschließen.
    pub include_snoozed: bool,
    pub include_archived: bool,
    /// Sichtbare Teams des Users; Issue-/Projekt-Notifications ohne sichtbares Team bleiben draußen
    /// (Zugriffsverlust blendet aus, docs/05 §6.1 Nr. 3).
    pub visible_teams: Vec<TeamId>,
}

/// Auswahl für „alle lesen/archivieren“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkFilter {
    /// Alles Nicht-Archivierte.
    All,
    /// Gruppen ohne ungelesene Notification.
    Read,
    /// Gruppen, deren Issue geschlossen ist (completed/canceled/duplicate).
    Completed,
}

/// Grund eines Issue-Abos (`issue_subscription.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionReason {
    Creator,
    Assignee,
    Mention,
    Comment,
    Manual,
    AddedByOther,
}

impl SubscriptionReason {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Creator => "creator",
            Self::Assignee => "assignee",
            Self::Mention => "mention",
            Self::Comment => "comment",
            Self::Manual => "manual",
            Self::AddedByOther => "addedByOther",
        }
    }

    /// Ob ein bestehendes `active = false` wieder aktiviert wird: nur Zuweisung, erneute
    /// Erwähnung oder manuelles Abonnieren (docs/05 §6.3); Kommentieren und die einmalige
    /// Ersteller-Regel nicht.
    pub const fn reactivates(&self) -> bool {
        !matches!(self, Self::Comment | Self::Creator)
    }
}

/// Ergebnis von `subscribe_issue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeOutcome {
    Created,
    Reactivated,
    Unchanged,
}

/// Ein Issue-Abo.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueSubscription {
    pub user_id: UserId,
    pub thread_comment_id: Option<CommentId>,
    pub active: bool,
    pub reason: String,
}

/// Für Benachrichtigungen relevante Fakten eines Issues.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueFacts {
    pub id: IssueId,
    pub team_id: TeamId,
    pub identifier: String,
    pub title: String,
    pub creator_id: Option<UserId>,
    pub assignee_id: Option<UserId>,
    pub project_id: Option<ProjectId>,
    pub cycle_id: Option<Uuid>,
    pub state_type: String,
    pub priority: i32,
    pub due_date: Option<NaiveDate>,
}

/// Fakten eines Kommentars (Elternobjekt, Thread). Genau eines der Elternfelder ist gesetzt.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentFacts {
    pub id: CommentId,
    pub author_id: Option<UserId>,
    pub issue_id: Option<IssueId>,
    pub project_id: Option<ProjectId>,
    pub project_milestone_id: Option<Uuid>,
    pub project_update_id: Option<Uuid>,
    pub initiative_id: Option<Uuid>,
    pub initiative_update_id: Option<Uuid>,
    pub document_id: Option<Uuid>,
    /// Elternkommentar (Thread-Wurzel), `None` bei Top-Level-Kommentaren.
    pub parent_id: Option<CommentId>,
}

/// Fakten einer Reaktion.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionFacts {
    pub user_id: UserId,
    pub emoji: String,
    pub issue_id: Option<IssueId>,
    pub comment_id: Option<CommentId>,
}

/// Fakten einer Issue-Relation.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationFacts {
    pub issue_id: IssueId,
    pub related_issue_id: IssueId,
    pub kind: String,
}

/// Fakten eines Projekt-Statusberichts.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectUpdateFacts {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub author_id: UserId,
}

/// Fakten eines Meilensteins.
#[derive(Debug, Clone, PartialEq)]
pub struct MilestoneFacts {
    pub id: Uuid,
    pub project_id: ProjectId,
}

/// Fakten eines Initiative-Statusberichts.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeUpdateFacts {
    pub id: Uuid,
    pub initiative_id: Uuid,
    pub author_id: UserId,
}

/// Fakten eines Dokuments: wer es sehen darf und wer es abonniert hat (docs/06, docs/05 §6.3).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentFacts {
    pub id: Uuid,
    pub creator_id: Option<UserId>,
    pub trashed: bool,
    /// Teams des Elternobjekts (Issue-Team, Projekt-Teams, Team); leer bei Initiative/Workspace.
    pub teams: Vec<TeamId>,
    /// Elternobjekt ohne Team (Initiative oder Workspace): nur Nicht-Gäste.
    pub workspace_only: bool,
}

/// Bündelungs-Kandidat: jüngste Notification desselben Akteurs in derselben Gruppe.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleTarget {
    pub id: NotificationId,
    pub kind: String,
    pub bundled_types: Vec<String>,
}

/// Zusammenführung eines Ereignisses in eine bestehende Notification.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleMerge {
    pub target: NotificationId,
    /// Neuer Haupttyp samt Kategorie (der spezifischere von beiden).
    pub kind: String,
    pub category: String,
    /// `true`: die Bezugs-Felder (Kommentar, Reaktion) des neuen Ereignisses ersetzen die alten.
    pub take_refs: bool,
    pub bundled_types: Vec<String>,
    pub is_priority: bool,
}

/// User-Daten für den Fan-out (Sichtbarkeit, Priorität).
#[derive(Debug, Clone, PartialEq)]
pub struct FanoutUser {
    pub id: UserId,
    pub role: UserRole,
    pub active: bool,
}

/// Benachrichtigungs-Einstellungen eines Users (Spalten am User plus Kanäle).
#[derive(Debug, Clone, PartialEq)]
pub struct UserNotificationPrefs {
    pub user_id: UserId,
    pub email: String,
    pub timezone: Option<String>,
    pub email_format: String,
    pub due_date_reminders: bool,
    pub priority_categories: Vec<String>,
    /// Weitere E-Mail-Schalter (`User.notificationSettings`).
    pub settings: Value,
}

/// Änderung der Einstellungen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserNotificationPrefsPatch {
    pub email_format: Option<String>,
    pub due_date_reminders: Option<bool>,
    pub priority_categories: Option<Vec<String>>,
    /// Wird mit den vorhandenen Settings zusammengeführt (flach).
    pub settings: Option<Value>,
}

/// Kanal-Einstellung (`notification_channel_preference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPreference {
    pub category: String,
    pub channel: String,
    pub enabled: bool,
}

/// Objekt, das abonniert wird (`notification_subscription.target_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Team,
    Project,
    Initiative,
    Cycle,
    Label,
}

impl ObjectKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Project => "project",
            Self::Initiative => "initiative",
            Self::Cycle => "cycle",
            Self::Label => "label",
        }
    }

    /// Spalte des Zielfelds.
    pub const fn column(&self) -> &'static str {
        match self {
            Self::Team => "team_id",
            Self::Project => "project_id",
            Self::Initiative => "initiative_id",
            Self::Cycle => "cycle_id",
            Self::Label => "label_id",
        }
    }
}

/// Objekt-Abo (docs/01 §5.5).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectSubscription {
    pub id: Uuid,
    pub subscriber_id: UserId,
    pub kind: ObjectKind,
    pub target_id: Uuid,
    pub context_view_type: Option<String>,
    pub types: Vec<String>,
    pub include_sub_initiative_updates: bool,
    pub active: bool,
}

/// Anlegen/Ändern eines Objekt-Abos (Upsert je Abonnent und Ziel/Kontext).
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertObjectSubscription {
    pub id: Uuid,
    pub subscriber_id: UserId,
    pub kind: ObjectKind,
    pub target_id: Uuid,
    pub context_view_type: Option<String>,
    pub types: Vec<String>,
    pub include_sub_initiative_updates: bool,
    pub active: bool,
    pub now: DateTime<Utc>,
}

/// Kandidat der Fälligkeits-Erinnerung.
#[derive(Debug, Clone, PartialEq)]
pub struct DueCandidate {
    pub issue_id: IssueId,
    pub team_id: TeamId,
    pub assignee_id: UserId,
    pub due_date: NaiveDate,
    pub timezone: Option<String>,
}

/// Persistenzzugriff des Bereichs `notification`.
#[async_trait]
pub trait NotificationRepo: Send {
    // --- Notifications ---

    /// Schreibt die Benachrichtigung (idempotent über `dedup_key`) und beendet ein laufendes
    /// Snooze der Gruppe (docs/05 §6.4 Regel 3). `false`, wenn sie schon existierte.
    async fn insert_notification(&mut self, _n: &NewNotification) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Ob zu `dedup_key` bereits eine Notification existiert oder das Ereignis in eine andere
    /// eingegangen ist (Idempotenz der Bündelung).
    async fn dedup_seen(&mut self, _user: UserId, _dedup_key: &str) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Jüngste nicht archivierte Notification von `actor` für `user` in der Gruppe, erzeugt nach `since`.
    async fn find_bundle_target(
        &mut self,
        _user: UserId,
        _group_key: &str,
        _actor: UserId,
        _since: DateTime<Utc>,
    ) -> AppResult<Option<BundleTarget>> {
        unimplemented_repo()
    }

    /// Führt das Ereignis `n` in die Ziel-Notification zusammen: Typ/Refs/`bundled_types` setzen,
    /// wieder ungelesen, laufendes Snooze der Gruppe beenden, `dedup_key` vermerken.
    async fn merge_notification(
        &mut self,
        _n: &NewNotification,
        _m: &BundleMerge,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    /// Eigene Notification (fremde → `None`).
    async fn get_notification(
        &mut self,
        _user: UserId,
        _id: NotificationId,
    ) -> AppResult<Option<NotificationView>> {
        unimplemented_repo()
    }

    /// Seite der Inbox, neueste zuerst (Cursor = `created_at`, `id`).
    async fn list_notifications(
        &mut self,
        _user: UserId,
        _query: &InboxQuery,
        _now: DateTime<Utc>,
        _page: &PageRequest,
    ) -> AppResult<Page<NotificationView>> {
        unimplemented_repo()
    }

    /// Anzahl ungelesener, nicht gesnoozter, nicht archivierter Gruppen.
    async fn count_unread_groups(
        &mut self,
        _user: UserId,
        _visible_teams: &[TeamId],
        _now: DateTime<Utc>,
    ) -> AppResult<i64> {
        unimplemented_repo()
    }

    /// Setzt `readAt` einer einzelnen Notification.
    async fn set_read(
        &mut self,
        _user: UserId,
        _id: NotificationId,
        _read: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    /// `readAt = now` für alle ungelesenen der Gruppe (Öffnen).
    async fn mark_group_read(
        &mut self,
        _user: UserId,
        _group_key: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Alles Nicht-Archivierte (bzw. gefiltert) als gelesen markieren; liefert die betroffenen IDs
    /// (für die Sync-Events).
    async fn mark_all_read(
        &mut self,
        _user: UserId,
        _visible_teams: &[TeamId],
        _filter: BulkFilter,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<NotificationId>> {
        unimplemented_repo()
    }

    /// Snooze der ganzen Gruppe; `until = None` beendet das Snooze.
    async fn snooze_group(
        &mut self,
        _user: UserId,
        _group_key: &str,
        _until: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Archiviert (`archive = true`) bzw. reaktiviert die Gruppe.
    async fn set_group_archived(
        &mut self,
        _user: UserId,
        _group_key: &str,
        _archive: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Archiviert alle (gefiltert) eigenen Notifications; liefert die betroffenen IDs.
    async fn archive_all(
        &mut self,
        _user: UserId,
        _visible_teams: &[TeamId],
        _filter: BulkFilter,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<NotificationId>> {
        unimplemented_repo()
    }

    // --- Jobs (mandantenübergreifend; nur unter `Store::begin_system`) ---

    /// Wiedervorlage: `unsnoozedAt = now`, `readAt = null` für abgelaufene Snoozes.
    async fn unsnooze_due(&mut self, _now: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Archiviert je User die ältesten Notifications über dem Limit
    /// (`limits.maxUnarchivedNotificationsPerUser`, Default 2000); gesnoozte zuletzt.
    async fn archive_over_cap(&mut self, _now: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Entfernt archivierte Notifications, die vor `before` archiviert wurden.
    async fn purge_archived(&mut self, _before: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }

    /// Ungelesene, nicht gemailte, nicht archivierte, nicht gesnoozte Notifications seit `since`
    /// aller Workspaces (E-Mail-Digest).
    async fn list_email_candidates(
        &mut self,
        _since: DateTime<Utc>,
        _now: DateTime<Utc>,
        _limit: i64,
    ) -> AppResult<Vec<NotificationView>> {
        unimplemented_repo()
    }

    /// Vermerkt den Versand (`emailedAt`).
    async fn mark_emailed(
        &mut self,
        _ids: &[NotificationId],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }

    /// Offene Issues mit Fälligkeit bis `due_until` und Assignee mit aktivierten
    /// Fälligkeits-Erinnerungen (alle Workspaces).
    async fn list_due_candidates(&mut self, _due_until: NaiveDate) -> AppResult<Vec<DueCandidate>> {
        unimplemented_repo()
    }

    /// Aktive Workspaces (für Jobs, die je Workspace arbeiten).
    async fn list_org_ids(&mut self) -> AppResult<Vec<Uuid>> {
        unimplemented_repo()
    }

    // --- Fakten für den Fan-out ---

    async fn issue_facts(&mut self, _id: IssueId) -> AppResult<Option<IssueFacts>> {
        unimplemented_repo()
    }
    /// UUID oder Kennung (`ENG-12`).
    async fn issue_facts_by_ref(&mut self, _reference: &str) -> AppResult<Option<IssueFacts>> {
        unimplemented_repo()
    }
    async fn comment_facts(&mut self, _id: CommentId) -> AppResult<Option<CommentFacts>> {
        unimplemented_repo()
    }
    async fn reaction_facts(&mut self, _id: Uuid) -> AppResult<Option<ReactionFacts>> {
        unimplemented_repo()
    }
    async fn relation_facts(&mut self, _id: Uuid) -> AppResult<Option<RelationFacts>> {
        unimplemented_repo()
    }
    async fn project_update_facts(&mut self, _id: Uuid) -> AppResult<Option<ProjectUpdateFacts>> {
        unimplemented_repo()
    }
    async fn milestone_facts(&mut self, _id: Uuid) -> AppResult<Option<MilestoneFacts>> {
        unimplemented_repo()
    }
    async fn initiative_update_facts(
        &mut self,
        _id: Uuid,
    ) -> AppResult<Option<InitiativeUpdateFacts>> {
        unimplemented_repo()
    }
    async fn document_facts(&mut self, _id: Uuid) -> AppResult<Option<DocumentFacts>> {
        unimplemented_repo()
    }
    /// Issues, die `issue` blockiert (`blocks`-Relation ausgehend).
    async fn blocked_issues(&mut self, _issue: IssueId) -> AppResult<Vec<IssueId>> {
        unimplemented_repo()
    }
    /// Anzahl noch offener Issues, die `issue` blockieren (`blocks`-Relation eingehend).
    async fn open_blockers(&mut self, _issue: IssueId) -> AppResult<i64> {
        unimplemented_repo()
    }
    /// Teams eines Projekts (`project_team`).
    async fn project_team_ids(&mut self, _project: ProjectId) -> AppResult<Vec<TeamId>> {
        unimplemented_repo()
    }
    /// Rolle und Aktiv-Flag der User.
    async fn fanout_users(&mut self, _ids: &[UserId]) -> AppResult<Vec<FanoutUser>> {
        unimplemented_repo()
    }

    // --- Issue-Abos ---

    /// Legt das Abo an bzw. reaktiviert es je nach `reason` (siehe [`SubscriptionReason::reactivates`]).
    async fn subscribe_issue(
        &mut self,
        _issue: IssueId,
        _user: UserId,
        _thread: Option<CommentId>,
        _reason: SubscriptionReason,
        _now: DateTime<Utc>,
    ) -> AppResult<SubscribeOutcome> {
        unimplemented_repo()
    }

    /// Setzt alle Abos (auch Thread-Abos) des Users am Issue auf `active = false`;
    /// `true`, wenn mindestens eines aktiv war.
    async fn unsubscribe_issue(
        &mut self,
        _issue: IssueId,
        _user: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Alle Abo-Datensätze des Issues.
    async fn list_issue_subscriptions(
        &mut self,
        _issue: IssueId,
    ) -> AppResult<Vec<IssueSubscription>> {
        unimplemented_repo()
    }

    /// Aktive Issue-Subscriber (ohne reine Thread-Abos).
    async fn issue_subscribers(&mut self, _issue: IssueId) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    /// Aktive Thread-Subscriber.
    async fn thread_subscribers(
        &mut self,
        _issue: IssueId,
        _thread: CommentId,
    ) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    /// Autoren der Wurzel und aller Antworten des Threads.
    async fn thread_participants(&mut self, _thread: CommentId) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }

    // --- Dokument-Abos (`document_subscription`) ---

    /// Legt das Abo an bzw. reaktiviert es je nach `reason` (wie bei Issues); `reason` darf nicht
    /// `Assignee` sein.
    async fn subscribe_document(
        &mut self,
        _document: Uuid,
        _user: UserId,
        _reason: SubscriptionReason,
        _now: DateTime<Utc>,
    ) -> AppResult<SubscribeOutcome> {
        unimplemented_repo()
    }

    /// Setzt das Abo auf `active = false`; `true`, wenn es aktiv war.
    async fn unsubscribe_document(
        &mut self,
        _document: Uuid,
        _user: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Aktive Abonnenten des Dokuments.
    async fn document_subscribers(&mut self, _document: Uuid) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }

    /// Ob `user` das Dokument aktiv abonniert hat.
    async fn is_document_subscriber(&mut self, _document: Uuid, _user: UserId) -> AppResult<bool> {
        unimplemented_repo()
    }

    // --- Objekt-Abos ---

    async fn upsert_object_subscription(
        &mut self,
        _sub: &UpsertObjectSubscription,
    ) -> AppResult<ObjectSubscription> {
        unimplemented_repo()
    }
    async fn list_object_subscriptions(
        &mut self,
        _user: UserId,
    ) -> AppResult<Vec<ObjectSubscription>> {
        unimplemented_repo()
    }
    /// Aktive Abonnenten des Objekts, deren Typen `ntype` enthalten.
    async fn object_subscribers(
        &mut self,
        _kind: ObjectKind,
        _target: Uuid,
        _ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    /// Abonnenten der Initiativen, denen das Projekt zugeordnet ist (Typ `ntype`).
    async fn initiative_subscribers_of_project(
        &mut self,
        _project: ProjectId,
        _ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    /// Abonnenten der Labels des Issues (Typ `ntype`).
    async fn label_subscribers_of_issue(
        &mut self,
        _issue: IssueId,
        _ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
    /// Teams, in denen ein Label gilt (`None` = Workspace-Label) und ob es existiert.
    async fn label_team(&mut self, _label: Uuid) -> AppResult<Option<Option<TeamId>>> {
        unimplemented_repo()
    }
    /// Team eines Cycles.
    async fn cycle_team(&mut self, _cycle: Uuid) -> AppResult<Option<TeamId>> {
        unimplemented_repo()
    }
    /// Ob die Initiative existiert.
    async fn initiative_exists(&mut self, _initiative: Uuid) -> AppResult<bool> {
        unimplemented_repo()
    }

    // --- Einstellungen ---

    async fn get_prefs(&mut self, _user: UserId) -> AppResult<Option<UserNotificationPrefs>> {
        unimplemented_repo()
    }
    async fn update_prefs(
        &mut self,
        _user: UserId,
        _patch: &UserNotificationPrefsPatch,
        _now: DateTime<Utc>,
    ) -> AppResult<UserNotificationPrefs> {
        unimplemented_repo()
    }
    async fn list_channel_preferences(
        &mut self,
        _user: UserId,
    ) -> AppResult<Vec<ChannelPreference>> {
        unimplemented_repo()
    }
    async fn set_channel_preference(
        &mut self,
        _user: UserId,
        _pref: &ChannelPreference,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Prioritäts-Kategorien der User (Fan-out, `isPriority`).
    async fn priority_categories(
        &mut self,
        _users: &[UserId],
    ) -> AppResult<Vec<(UserId, Vec<String>)>> {
        unimplemented_repo()
    }
}
