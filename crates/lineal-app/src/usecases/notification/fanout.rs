//! Notification-Fan-out (docs/10 §4.2, docs/05 §6): Outbox-Event → Empfänger → Notifications.
//!
//! Ablauf je Event: Abos nachführen (Creator, Assignee, Mention, Kommentar), Kandidaten sammeln
//! (der spezifischste Typ je Empfänger gewinnt), Self-Suppression, Sichtbarkeit je Empfänger
//! prüfen, Notifications idempotent schreiben (`dedup_key`).

use super::kinds::{category_of, is_always_priority, rank_of};
use super::views::{ViewIssueEvent, ViewSubscriptionSource};
use crate::app::App;
use crate::context::Actor;
use crate::error::AppResult;
use crate::policy::{ActorView, TeamTree, compute_visible_team_ids};
use crate::ports::Tx;
use crate::ports::repos::notification::{
    BundleMerge, DocumentFacts, IssueFacts, NewNotification, ObjectKind, SubscriptionReason,
};
use crate::ports::repos::{NewEvent, StoredEvent};
use chrono::{DateTime, Duration, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::events::{DomainEvent, EventOrigin, IssuePrevious};
use lineal_domain::ids::{CommentId, IssueId, NotificationId, ProjectId, TeamId, UserId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

const CLOSED: [&str; 3] = ["completed", "canceled", "duplicate"];

/// Fenster der Bündelung gleichartiger Notifications (docs/05 §6.6).
const BUNDLE_WINDOW_SECS: i64 = 60;
/// Höchstzahl der vermerkten gebündelten Typen je Notification.
const MAX_BUNDLED_TYPES: usize = 20;

/// Bezugsobjekt einer Notification.
#[derive(Debug, Clone, Default)]
pub(crate) struct Refs {
    pub target_type: &'static str,
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
}

/// Ob zwei Typen einander aufheben.
fn is_opposite(a: &str, b: &str) -> bool {
    matches!(
        (a, b),
        ("issueAssignedToYou", "issueUnassignedFromYou")
            | ("issueUnassignedFromYou", "issueAssignedToYou")
            | ("issueSubscribed", "issueUnsubscribed")
            | ("issueUnsubscribed", "issueSubscribed")
            | ("documentSubscribed", "documentUnsubscribed")
            | ("documentUnsubscribed", "documentSubscribed")
    )
}

/// Gruppenschlüssel einer neuen Notification (wie die generierte Spalte `group_key`).
fn group_key_of(n: &NewNotification) -> Option<String> {
    match n.target_type.as_str() {
        "issue" => n.issue_id.map(|i| format!("issue:{}", i.0)),
        "project" => n.project_id.map(|i| format!("project:{}", i.0)),
        "initiative" => n.initiative_id.map(|i| format!("initiative:{i}")),
        "document" => n.document_id.map(|i| format!("document:{i}")),
        _ => None,
    }
}

/// Sammelt Kandidaten für genau ein Event bzw. eine Erinnerung.
pub(crate) struct Fan<'a> {
    tx: &'a mut dyn Tx,
    tree: TeamTree,
    visible: HashMap<UserId, Option<(BTreeSet<TeamId>, UserRole)>>,
    now: DateTime<Utc>,
    pub refs: Refs,
    /// Sichtbar, wenn der Empfänger mindestens eines dieser Teams sieht (leer = immer).
    pub teams: Vec<TeamId>,
    /// Bezugsobjekt ohne Team (Initiative, Workspace-Dokument): Gäste sehen es nie.
    pub workspace_only: bool,
    pub actor: Option<UserId>,
    cands: BTreeMap<UserId, &'static str>,
    /// Zwischenspeicher: erlaubt der Workspace Freigaben?
    sharing_enabled: Option<bool>,
    /// Quelle für View-Abos (WP31); `None` = keine View-Benachrichtigungen.
    views: Option<&'a dyn ViewSubscriptionSource>,
    /// Issues, die durch das Ereignis frei geworden sind (`issueUnblocked`).
    unblocked: Vec<IssueId>,
    /// Ersetzt das Ereignis-Präfix des Dedup-Schlüssels: Objektänderungen (Projekt, Initiative,
    /// Meilenstein, Bericht) tragen keine Änderungsliste; je Empfänger und Typ gibt es dann
    /// höchstens eine Benachrichtigung je Objekt.
    dedup_override: Option<String>,
}

impl<'a> Fan<'a> {
    pub async fn new(
        tx: &'a mut dyn Tx,
        now: DateTime<Utc>,
        actor: Option<UserId>,
    ) -> AppResult<Self> {
        let tree = TeamTree::new(tx.teams().list_all().await?);
        Ok(Self {
            tx,
            tree,
            visible: HashMap::new(),
            now,
            refs: Refs::default(),
            teams: Vec::new(),
            workspace_only: false,
            actor,
            cands: BTreeMap::new(),
            sharing_enabled: None,
            views: None,
            unblocked: Vec::new(),
            dedup_override: None,
        })
    }

    /// Hängt die Quelle für View-Abos ein.
    pub fn with_views(mut self, views: Option<&'a dyn ViewSubscriptionSource>) -> Self {
        self.views = views;
        self
    }

    /// Ob `user` aktiv ist und mindestens eines der Teams sieht (`teams` leer → ja).
    pub async fn can_see_any(&mut self, user: UserId, teams: &[TeamId]) -> AppResult<bool> {
        if !self.visible.contains_key(&user) {
            let set = match self.tx.notifications().fanout_users(&[user]).await?.pop() {
                Some(u) if u.active => {
                    let memberships = self.tx.teams().list_memberships_of_user(user).await?;
                    let view = ActorView::new(
                        Actor::User {
                            id: user,
                            role: u.role,
                        },
                        &memberships,
                    );
                    Some((compute_visible_team_ids(&view, &self.tree), u.role))
                }
                _ => None,
            };
            self.visible.insert(user, set);
        }
        let workspace_only = self.workspace_only;
        Ok(match self.visible.get(&user) {
            Some(Some((_, role))) if workspace_only && *role == UserRole::Guest => false,
            Some(Some((set, _))) => teams.is_empty() || teams.iter().any(|t| set.contains(t)),
            _ => false,
        })
    }

    /// Wie [`Fan::can_see_any`] für ein Issue; zusätzlich sieht es, wem es freigegeben wurde
    /// (`issue_share`, docs/09 §5.6).
    pub async fn can_see_issue(
        &mut self,
        user: UserId,
        issue: IssueId,
        team: TeamId,
    ) -> AppResult<bool> {
        if self.can_see_any(user, &[team]).await? {
            return Ok(true);
        }
        let active = matches!(self.visible.get(&user), Some(Some(_)));
        if !active || !self.sharing_enabled().await? {
            return Ok(false);
        }
        self.tx.documents().issue_shared_with(issue, user).await
    }

    /// Ob der Workspace Freigaben erlaubt; abgeschaltet wirken bestehende Freigaben nicht
    /// (dieselbe Regel wie `resolve::issue::share_recipient`).
    async fn sharing_enabled(&mut self) -> AppResult<bool> {
        if let Some(v) = self.sharing_enabled {
            return Ok(v);
        }
        let v = match self.tx.orgs().get().await {
            Ok(Some(org)) => {
                crate::policy::OrgPolicy::from_json(&org.security_settings, &org.limits)
                    .limits
                    .issue_sharing_enabled
            }
            Ok(None) => false,
            Err(e) if e.message == "not implemented" => false,
            Err(e) => return Err(e),
        };
        self.sharing_enabled = Some(v);
        Ok(v)
    }

    /// Fügt einen Kandidaten hinzu; bei mehreren Gründen gewinnt der spezifischste Typ.
    pub fn add(&mut self, user: UserId, kind: &'static str) {
        match self.cands.get(&user) {
            Some(old) if rank_of(old) <= rank_of(kind) => {}
            _ => {
                self.cands.insert(user, kind);
            }
        }
    }

    pub fn add_all(&mut self, users: impl IntoIterator<Item = UserId>, kind: &'static str) {
        for u in users {
            self.add(u, kind);
        }
    }

    /// Bereitet eine neue Erinnerung zu einem Issue vor (Fälligkeits-Job).
    pub fn reset_for_issue_reminder(&mut self, issue: IssueId, team: TeamId) {
        self.refs = Refs {
            target_type: "issue",
            issue_id: Some(issue),
            team_id: Some(team),
            ..Refs::default()
        };
        self.teams = vec![team];
        self.workspace_only = false;
        self.cands.clear();
    }

    /// Bereitet eine Benachrichtigung zu einem Dokument vor (Erwähnungen, WP34); sichtbar für
    /// Empfänger, die mindestens eines der Teams des Elternobjekts sehen (leer = alle aktiven).
    pub fn reset_for_document(&mut self, document: Uuid, teams: Vec<TeamId>) {
        self.refs = Refs {
            target_type: "document",
            document_id: Some(document),
            ..Refs::default()
        };
        self.teams = teams;
        self.cands.clear();
    }

    /// Issue als Bezugsobjekt.
    pub fn target_issue(&mut self, f: &IssueFacts) {
        self.refs.target_type = "issue";
        self.refs.issue_id = Some(f.id);
        self.refs.team_id = Some(f.team_id);
        self.teams = vec![f.team_id];
    }

    async fn target_project(&mut self, project: ProjectId) -> AppResult<()> {
        self.refs.target_type = "project";
        self.refs.project_id = Some(project);
        self.teams = self.tx.notifications().project_team_ids(project).await?;
        // Projekte ohne Team gehören dem Workspace: nichts für Gäste.
        self.workspace_only = self.teams.is_empty();
        Ok(())
    }

    fn target_initiative(&mut self, initiative: Uuid) {
        self.refs.target_type = "initiative";
        self.refs.initiative_id = Some(initiative);
        self.teams = Vec::new();
        self.workspace_only = true;
    }

    /// Abo nachführen, aber nur für User, die das Issue sehen.
    async fn subscribe(
        &mut self,
        f: &IssueFacts,
        user: UserId,
        thread: Option<CommentId>,
        reason: SubscriptionReason,
    ) -> AppResult<()> {
        if self.can_see_issue(user, f.id, f.team_id).await? {
            self.tx
                .notifications()
                .subscribe_issue(f.id, user, thread, reason, self.now)
                .await?;
        }
        Ok(())
    }

    /// Falls der Emitter keinen Akteur gesetzt hat, gilt bei User-Events der Urheber.
    fn fallback_actor(&mut self, origin: EventOrigin, author: Option<UserId>) {
        if self.actor.is_none() && origin == EventOrigin::User {
            self.actor = author;
        }
    }

    /// Leitet aus dem Event Abos und Kandidaten ab. `false`, wenn das Bezugsobjekt fehlt oder das
    /// Event keine Notifications auslöst.
    pub async fn plan(&mut self, ev: &StoredEvent) -> AppResult<bool> {
        match &ev.event {
            DomainEvent::IssueCreated {
                issue_id,
                mentioned_user_ids,
                ..
            } => self.issue_created(ev, *issue_id, mentioned_user_ids).await,
            DomainEvent::IssueUpdated {
                issue_id,
                changed,
                mentioned_user_ids,
                previous,
                ..
            } => {
                self.issue_updated(*issue_id, changed, mentioned_user_ids, previous.as_ref())
                    .await
            }
            DomainEvent::CommentCreated {
                comment_id,
                mentioned_user_ids,
            } => {
                self.comment(ev, *comment_id, mentioned_user_ids, true)
                    .await
            }
            DomainEvent::CommentUpdated {
                comment_id,
                mentioned_user_ids,
            } => {
                self.comment(ev, *comment_id, mentioned_user_ids, false)
                    .await
            }
            DomainEvent::ReactionCreated { reaction_id } => self.reaction(ev, reaction_id.0).await,
            DomainEvent::IssueRelationCreated { relation_id, .. } => {
                self.relation_created(relation_id.0).await
            }
            DomainEvent::IssueSubscriptionCreated { issue_id, user_id } => {
                self.subscription_event(*issue_id, *user_id, "issueSubscribed")
                    .await
            }
            DomainEvent::IssueSubscriptionRemoved { issue_id, user_id } => {
                self.subscription_event(*issue_id, *user_id, "issueUnsubscribed")
                    .await
            }
            DomainEvent::ProjectUpdateCreated {
                update_id,
                project_id,
            } => {
                self.project_update_created(ev, update_id.0, *project_id)
                    .await
            }
            DomainEvent::ProjectUpdateUpdated {
                update_id,
                project_id,
            } => {
                self.project_update_mentions(ev, update_id.0, *project_id)
                    .await
            }
            DomainEvent::ProjectCreated { project_id } => {
                self.project_changed(*project_id, true).await
            }
            DomainEvent::ProjectUpdated { project_id } => {
                self.project_changed(*project_id, false).await
            }
            DomainEvent::ProjectMilestoneCreated { milestone_id, .. }
            | DomainEvent::ProjectMilestoneUpdated { milestone_id, .. } => {
                self.milestone_changed(milestone_id.0).await
            }
            DomainEvent::InitiativeCreated { initiative_id } => {
                self.initiative_changed(initiative_id.0, true).await
            }
            DomainEvent::InitiativeUpdated { initiative_id } => {
                self.initiative_changed(initiative_id.0, false).await
            }
            DomainEvent::InitiativeUpdateCreated { update_id, .. } => {
                self.initiative_update(ev, update_id.0, true).await
            }
            DomainEvent::InitiativeUpdateUpdated { update_id, .. } => {
                self.initiative_update(ev, update_id.0, false).await
            }
            DomainEvent::IssueSlaHighRisk { issue_id } => {
                self.sla(*issue_id, "issueSlaHighRisk").await
            }
            DomainEvent::IssueSlaBreached { issue_id } => {
                self.sla(*issue_id, "issueSlaBreached").await
            }
            _ => Ok(false),
        }
    }

    async fn issue_created(
        &mut self,
        ev: &StoredEvent,
        issue: IssueId,
        mentioned: &[UserId],
    ) -> AppResult<bool> {
        let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
            return Ok(false);
        };
        self.target_issue(&f);
        self.fallback_actor(ev.origin, f.creator_id);
        if let Some(creator) = f.creator_id.or(ev.actor_id) {
            self.subscribe(&f, creator, None, SubscriptionReason::Creator)
                .await?;
        }
        if let Some(a) = f.assignee_id {
            self.subscribe(&f, a, None, SubscriptionReason::Assignee)
                .await?;
            self.add(a, "issueAssignedToYou");
            if f.priority == 1 {
                self.add(a, "issuePriorityUrgent");
            }
        }
        for m in mentioned {
            self.subscribe(&f, *m, None, SubscriptionReason::Mention)
                .await?;
            self.add(*m, "issueMention");
        }
        let notifications = self.tx.notifications();
        let mut created = notifications
            .object_subscribers(ObjectKind::Team, f.team_id.0, "issueCreated")
            .await?;
        if let Some(p) = f.project_id {
            created.extend(
                notifications
                    .object_subscribers(ObjectKind::Project, p.0, "issueCreated")
                    .await?,
            );
        }
        created.extend(
            notifications
                .label_subscribers_of_issue(f.id, "issueCreated")
                .await?,
        );
        let triage = if f.state_type == "triage" {
            notifications
                .object_subscribers(ObjectKind::Team, f.team_id.0, "issueAddedToTriage")
                .await?
        } else {
            Vec::new()
        };
        self.add_all(created, "issueCreated");
        self.add_all(triage, "issueAddedToTriage");
        self.add_view_recipients(&f, true, &[], None).await?;
        Ok(true)
    }

    /// Abonnenten abonnierter Views (WP31, siehe [`ViewSubscriptionSource`]).
    async fn add_view_recipients(
        &mut self,
        f: &IssueFacts,
        is_new: bool,
        changed: &[String],
        previous: Option<&IssuePrevious>,
    ) -> AppResult<()> {
        let Some(views) = self.views else {
            return Ok(());
        };
        let found = views
            .recipients(
                &mut *self.tx,
                &ViewIssueEvent {
                    now: self.now,
                    issue: f,
                    is_new,
                    changed,
                    previous,
                },
            )
            .await?;
        for (user, kind) in found {
            self.add(user, kind);
        }
        Ok(())
    }

    async fn issue_updated(
        &mut self,
        issue: IssueId,
        changed: &[String],
        mentioned: &[UserId],
        previous: Option<&IssuePrevious>,
    ) -> AppResult<bool> {
        let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
            return Ok(false);
        };
        self.target_issue(&f);
        let has = |name: &str| changed.iter().any(|c| c == name);
        for m in mentioned {
            self.subscribe(&f, *m, None, SubscriptionReason::Mention)
                .await?;
            self.add(*m, "issueMention");
        }
        if has("assigneeId")
            && let Some(a) = f.assignee_id
        {
            self.subscribe(&f, a, None, SubscriptionReason::Assignee)
                .await?;
            self.add(a, "issueAssignedToYou");
        }
        // Der bisherige Assignee erfährt von der Abgabe (docs/05 §6.2 `issueUnassignedFromYou`).
        if has("assigneeId")
            && let Some(old) = previous.and_then(|p| p.assignee_id)
            && Some(old) != f.assignee_id
        {
            self.add(old, "issueUnassignedFromYou");
        }
        if has("priority")
            && f.priority == 1
            && let Some(a) = f.assignee_id
        {
            self.add(a, "issuePriorityUrgent");
        }
        if has("teamId") {
            // Abos von Usern, die das neue Team nicht sehen dürfen, entfallen (docs/05 §6.3).
            for s in self.tx.notifications().issue_subscribers(f.id).await? {
                if !self.can_see_issue(s, f.id, f.team_id).await? {
                    self.tx
                        .notifications()
                        .unsubscribe_issue(f.id, s, self.now)
                        .await?;
                }
            }
        }
        if has("stateId") {
            let closed = CLOSED.contains(&f.state_type.as_str());
            let was_closed = previous
                .and_then(|p| p.state_type.as_deref())
                .is_some_and(|t| CLOSED.contains(&t));
            if closed {
                // Wer von diesem Issue blockiert wurde und nun keinen offenen Blocker mehr hat.
                for blocked in self.tx.notifications().blocked_issues(f.id).await? {
                    if self.tx.notifications().open_blockers(blocked).await? == 0 {
                        self.unblocked.push(blocked);
                    }
                }
            }
            if was_closed && !closed {
                let subs = self.tx.notifications().issue_subscribers(f.id).await?;
                self.add_all(subs, "issueReopened");
            }
            let notifications = self.tx.notifications();
            let mut subs = Vec::new();
            let mut all = notifications
                .object_subscribers(ObjectKind::Team, f.team_id.0, "issueStatusChangedAll")
                .await?;
            if let Some(c) = f.cycle_id {
                all.extend(
                    notifications
                        .object_subscribers(ObjectKind::Cycle, c, "issueStatusChangedAll")
                        .await?,
                );
            }
            let mut triage = Vec::new();
            if closed {
                subs.extend(notifications.issue_subscribers(f.id).await?);
                subs.extend(
                    notifications
                        .object_subscribers(ObjectKind::Team, f.team_id.0, "issueStatusChanged")
                        .await?,
                );
                if let Some(p) = f.project_id {
                    subs.extend(
                        notifications
                            .object_subscribers(ObjectKind::Project, p.0, "issueStatusChanged")
                            .await?,
                    );
                }
                if let Some(c) = f.cycle_id {
                    subs.extend(
                        notifications
                            .object_subscribers(ObjectKind::Cycle, c, "issueStatusChanged")
                            .await?,
                    );
                }
            } else if f.state_type == "triage" {
                triage = notifications
                    .object_subscribers(ObjectKind::Team, f.team_id.0, "issueAddedToTriage")
                    .await?;
            }
            self.add_all(subs, "issueStatusChanged");
            self.add_all(all, "issueStatusChangedAll");
            self.add_all(triage, "issueAddedToTriage");
        }
        self.add_view_recipients(&f, false, changed, previous)
            .await?;
        Ok(true)
    }

    async fn comment(
        &mut self,
        ev: &StoredEvent,
        comment: CommentId,
        mentioned: &[UserId],
        is_new: bool,
    ) -> AppResult<bool> {
        let Some(c) = self.tx.notifications().comment_facts(comment).await? else {
            return Ok(false);
        };
        self.fallback_actor(ev.origin, c.author_id);
        self.refs.comment_id = Some(c.id);
        self.refs.parent_comment_id = c.parent_id;
        let thread = c.parent_id.unwrap_or(c.id);
        if let Some(issue) = c.issue_id {
            let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
                return Ok(false);
            };
            self.target_issue(&f);
            if is_new && let Some(author) = c.author_id {
                self.subscribe(&f, author, None, SubscriptionReason::Comment)
                    .await?;
            }
            for m in mentioned {
                // Erwähnung in einer Antwort abonniert nur den Thread (docs/05 §6.3).
                let scope = c.parent_id.map(|_| thread);
                self.subscribe(&f, *m, scope, SubscriptionReason::Mention)
                    .await?;
                self.add(*m, "issueCommentMention");
            }
            if is_new {
                let notifications = self.tx.notifications();
                let mut recipients = notifications.issue_subscribers(f.id).await?;
                if c.parent_id.is_some() {
                    recipients.extend(notifications.thread_subscribers(f.id, thread).await?);
                    recipients.extend(notifications.thread_participants(thread).await?);
                }
                self.add_all(recipients, "issueNewComment");
            }
            return Ok(true);
        }
        if let Some(project) = c.project_id {
            self.target_project(project).await?;
            self.add_all(mentioned.iter().copied(), "projectCommentMention");
            if is_new {
                let notifications = self.tx.notifications();
                let mut recipients = notifications
                    .object_subscribers(ObjectKind::Project, project.0, "projectNewComment")
                    .await?;
                if c.parent_id.is_some() {
                    recipients.extend(notifications.thread_participants(thread).await?);
                }
                self.add_all(recipients, "projectNewComment");
            }
            return Ok(true);
        }
        if let Some(milestone) = c.project_milestone_id {
            let Some(m) = self.tx.notifications().milestone_facts(milestone).await? else {
                return Ok(false);
            };
            self.target_project(m.project_id).await?;
            self.refs.project_milestone_id = Some(m.id);
            self.add_all(mentioned.iter().copied(), "projectMilestoneCommentMention");
            if is_new {
                let notifications = self.tx.notifications();
                let mut recipients = notifications
                    .object_subscribers(ObjectKind::Project, m.project_id.0, "projectNewComment")
                    .await?;
                if c.parent_id.is_some() {
                    recipients.extend(notifications.thread_participants(thread).await?);
                }
                self.add_all(recipients, "projectMilestoneNewComment");
            }
            return Ok(true);
        }
        if let Some(update) = c.project_update_id {
            let Some(u) = self.tx.notifications().project_update_facts(update).await? else {
                return Ok(false);
            };
            self.target_project(u.project_id).await?;
            self.refs.project_update_id = Some(u.id);
            self.add_all(mentioned.iter().copied(), "projectUpdateCommentMention");
            if is_new {
                let mut recipients = vec![u.author_id];
                if c.parent_id.is_some() {
                    recipients.extend(self.tx.notifications().thread_participants(thread).await?);
                }
                self.add_all(recipients, "projectUpdateNewComment");
            }
            return Ok(true);
        }
        if let Some(initiative) = c.initiative_id {
            if !self
                .tx
                .notifications()
                .initiative_exists(initiative)
                .await?
            {
                return Ok(false);
            }
            self.target_initiative(initiative);
            self.add_all(mentioned.iter().copied(), "initiativeCommentMention");
            if is_new {
                let notifications = self.tx.notifications();
                let mut recipients = notifications
                    .object_subscribers(ObjectKind::Initiative, initiative, "initiativeNewComment")
                    .await?;
                if c.parent_id.is_some() {
                    recipients.extend(notifications.thread_participants(thread).await?);
                }
                self.add_all(recipients, "initiativeNewComment");
            }
            return Ok(true);
        }
        if let Some(update) = c.initiative_update_id {
            let Some(u) = self
                .tx
                .notifications()
                .initiative_update_facts(update)
                .await?
            else {
                return Ok(false);
            };
            self.target_initiative(u.initiative_id);
            self.refs.initiative_update_id = Some(u.id);
            self.add_all(mentioned.iter().copied(), "initiativeUpdateCommentMention");
            if is_new {
                let mut recipients = vec![u.author_id];
                if c.parent_id.is_some() {
                    recipients.extend(self.tx.notifications().thread_participants(thread).await?);
                }
                self.add_all(recipients, "initiativeUpdateNewComment");
            }
            return Ok(true);
        }
        if let Some(document) = c.document_id {
            let Some(d) = self.tx.notifications().document_facts(document).await? else {
                return Ok(false);
            };
            if d.trashed {
                return Ok(false);
            }
            self.refs.target_type = "document";
            self.refs.document_id = Some(d.id);
            self.teams = d.teams.clone();
            self.workspace_only = d.workspace_only;
            if is_new && let Some(author) = c.author_id {
                self.subscribe_document(&d, author, SubscriptionReason::Comment)
                    .await?;
            }
            for m in mentioned {
                self.subscribe_document(&d, *m, SubscriptionReason::Mention)
                    .await?;
            }
            self.add_all(mentioned.iter().copied(), "documentCommentMention");
            if is_new {
                let notifications = self.tx.notifications();
                let mut recipients = notifications.document_subscribers(d.id).await?;
                if c.parent_id.is_some() {
                    recipients.extend(notifications.thread_participants(thread).await?);
                }
                self.add_all(recipients, "documentNewComment");
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Dokument-Abo nachführen, aber nur für User, die das Dokument sehen (Gäste nur bei Teambezug).
    async fn subscribe_document(
        &mut self,
        d: &DocumentFacts,
        user: UserId,
        reason: SubscriptionReason,
    ) -> AppResult<()> {
        if self.can_see_any(user, &d.teams).await? {
            self.tx
                .notifications()
                .subscribe_document(d.id, user, reason, self.now)
                .await?;
        }
        Ok(())
    }

    async fn reaction(&mut self, ev: &StoredEvent, reaction: Uuid) -> AppResult<bool> {
        let Some(r) = self.tx.notifications().reaction_facts(reaction).await? else {
            return Ok(false);
        };
        self.fallback_actor(ev.origin, Some(r.user_id));
        self.refs.reaction_emoji = Some(r.emoji.clone());
        if let Some(cid) = r.comment_id {
            let Some(c) = self.tx.notifications().comment_facts(cid).await? else {
                return Ok(false);
            };
            self.refs.comment_id = Some(c.id);
            self.refs.parent_comment_id = c.parent_id;
            if let Some(issue) = c.issue_id {
                let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
                    return Ok(false);
                };
                self.target_issue(&f);
                if let Some(a) = c.author_id {
                    self.add(a, "issueCommentReaction");
                }
                return Ok(true);
            }
            if let Some(project) = c.project_id {
                self.target_project(project).await?;
                if let Some(a) = c.author_id {
                    self.add(a, "projectCommentReaction");
                }
                return Ok(true);
            }
            if let Some(milestone) = c.project_milestone_id {
                let Some(m) = self.tx.notifications().milestone_facts(milestone).await? else {
                    return Ok(false);
                };
                self.target_project(m.project_id).await?;
                self.refs.project_milestone_id = Some(m.id);
                if let Some(a) = c.author_id {
                    self.add(a, "projectMilestoneCommentReaction");
                }
                return Ok(true);
            }
            if let Some(update) = c.project_update_id {
                let Some(u) = self.tx.notifications().project_update_facts(update).await? else {
                    return Ok(false);
                };
                self.target_project(u.project_id).await?;
                self.refs.project_update_id = Some(u.id);
                if let Some(a) = c.author_id {
                    self.add(a, "projectUpdateCommentReaction");
                }
                return Ok(true);
            }
            if let Some(initiative) = c.initiative_id {
                if !self
                    .tx
                    .notifications()
                    .initiative_exists(initiative)
                    .await?
                {
                    return Ok(false);
                }
                self.target_initiative(initiative);
                if let Some(a) = c.author_id {
                    self.add(a, "initiativeCommentReaction");
                }
                return Ok(true);
            }
            if let Some(update) = c.initiative_update_id {
                let Some(u) = self
                    .tx
                    .notifications()
                    .initiative_update_facts(update)
                    .await?
                else {
                    return Ok(false);
                };
                self.target_initiative(u.initiative_id);
                self.refs.initiative_update_id = Some(u.id);
                if let Some(a) = c.author_id {
                    self.add(a, "initiativeUpdateCommentReaction");
                }
                return Ok(true);
            }
            if let Some(document) = c.document_id {
                let Some(d) = self.tx.notifications().document_facts(document).await? else {
                    return Ok(false);
                };
                if d.trashed {
                    return Ok(false);
                }
                self.refs.target_type = "document";
                self.refs.document_id = Some(d.id);
                self.teams = d.teams.clone();
                self.workspace_only = d.workspace_only;
                if let Some(a) = c.author_id {
                    self.add(a, "documentCommentReaction");
                }
                return Ok(true);
            }
            return Ok(false);
        }
        if let Some(update) = r.project_update_id {
            let Some(u) = self.tx.notifications().project_update_facts(update).await? else {
                return Ok(false);
            };
            self.target_project(u.project_id).await?;
            self.refs.project_update_id = Some(u.id);
            self.add(u.author_id, "projectUpdateReaction");
            return Ok(true);
        }
        if let Some(update) = r.initiative_update_id {
            let Some(u) = self
                .tx
                .notifications()
                .initiative_update_facts(update)
                .await?
            else {
                return Ok(false);
            };
            self.target_initiative(u.initiative_id);
            self.refs.initiative_update_id = Some(u.id);
            self.add(u.author_id, "initiativeUpdateReaction");
            return Ok(true);
        }
        if let Some(issue) = r.issue_id {
            let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
                return Ok(false);
            };
            self.target_issue(&f);
            if let Some(creator) = f.creator_id {
                self.add(creator, "issueEmojiReaction");
            }
            return Ok(true);
        }
        Ok(false)
    }

    async fn relation_created(&mut self, relation: Uuid) -> AppResult<bool> {
        let Some(r) = self.tx.notifications().relation_facts(relation).await? else {
            return Ok(false);
        };
        if r.kind != "blocks" {
            return Ok(false);
        }
        // `issue` blockiert `related_issue`: Benachrichtigt wird das blockierte Issue.
        let Some(f) = self
            .tx
            .notifications()
            .issue_facts(r.related_issue_id)
            .await?
        else {
            return Ok(false);
        };
        self.target_issue(&f);
        let mut recipients = self.tx.notifications().issue_subscribers(f.id).await?;
        recipients.extend(f.assignee_id);
        self.add_all(recipients, "issueBlocking");
        Ok(true)
    }

    async fn subscription_event(
        &mut self,
        issue: IssueId,
        user: UserId,
        kind: &'static str,
    ) -> AppResult<bool> {
        let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
            return Ok(false);
        };
        self.target_issue(&f);
        self.add(user, kind);
        Ok(true)
    }

    async fn project_update_created(
        &mut self,
        ev: &StoredEvent,
        update: Uuid,
        project: ProjectId,
    ) -> AppResult<bool> {
        let Some(u) = self.tx.notifications().project_update_facts(update).await? else {
            return Ok(false);
        };
        self.fallback_actor(ev.origin, Some(u.author_id));
        self.target_project(project).await?;
        self.refs.project_update_id = Some(u.id);
        self.dedup_override = Some(format!("once:projectUpdate:{}", u.id));
        let mentioned = self.mention_users(&u.body).await?;
        self.add_all(mentioned, "projectUpdateMention");
        let notifications = self.tx.notifications();
        let mut recipients = notifications
            .object_subscribers(ObjectKind::Project, project.0, "projectUpdateCreated")
            .await?;
        recipients.extend(
            notifications
                .initiative_subscribers_of_project(project, "projectUpdateCreated")
                .await?,
        );
        self.add_all(recipients, "projectUpdateCreated");
        Ok(true)
    }

    /// Erwähnte User im Text (aktive User; die Sichtbarkeit prüft `deliver` je Empfänger).
    async fn mention_users(&mut self, text: &str) -> AppResult<Vec<UserId>> {
        super::super::issue::mentions::resolve_handles(&mut *self.tx, text).await
    }

    /// Erwähnungen in einem geänderten Statusbericht (nur neue, je Bericht einmalig).
    async fn project_update_mentions(
        &mut self,
        ev: &StoredEvent,
        update: Uuid,
        project: ProjectId,
    ) -> AppResult<bool> {
        let Some(u) = self.tx.notifications().project_update_facts(update).await? else {
            return Ok(false);
        };
        self.fallback_actor(ev.origin, Some(u.author_id));
        self.target_project(project).await?;
        self.refs.project_update_id = Some(u.id);
        self.dedup_override = Some(format!("once:projectUpdate:{}", u.id));
        let mentioned = self.mention_users(&u.body).await?;
        self.add_all(mentioned, "projectUpdateMention");
        Ok(true)
    }

    /// Projekt angelegt oder geändert: neuer Lead (`projectAddedAsLead`) und Erwähnungen in
    /// Beschreibung/Inhalt (`projectMention`); je Projekt und Empfänger einmalig.
    ///
    /// Lead und Owner nur bei der Anlage: `ProjectUpdated`/`InitiativeUpdated` tragen keine
    /// Änderungsliste, ein späterer Wechsel ist daher nicht erkennbar (und eine Selbstzuweisung
    /// würde sonst bei der nächsten fremden Änderung gemeldet).
    async fn project_changed(&mut self, project: ProjectId, is_new: bool) -> AppResult<bool> {
        let Some(p) = self.tx.notifications().project_facts(project).await? else {
            return Ok(false);
        };
        self.target_project(project).await?;
        self.dedup_override = Some(format!("once:project:{}", project.0));
        if is_new && let Some(lead) = p.lead_id {
            self.add(lead, "projectAddedAsLead");
        }
        let mentioned = self.mention_users(&p.text).await?;
        self.add_all(mentioned, "projectMention");
        Ok(true)
    }

    async fn milestone_changed(&mut self, milestone: Uuid) -> AppResult<bool> {
        let Some(m) = self.tx.notifications().milestone_facts(milestone).await? else {
            return Ok(false);
        };
        self.target_project(m.project_id).await?;
        self.refs.project_milestone_id = Some(m.id);
        self.dedup_override = Some(format!("once:milestone:{}", m.id));
        let mentioned = self.mention_users(&m.description).await?;
        self.add_all(mentioned, "projectMilestoneMention");
        Ok(true)
    }

    /// Initiative angelegt oder geändert: neuer Owner und Erwähnungen.
    async fn initiative_changed(&mut self, initiative: Uuid, is_new: bool) -> AppResult<bool> {
        let Some(i) = self.tx.notifications().initiative_facts(initiative).await? else {
            return Ok(false);
        };
        self.target_initiative(initiative);
        self.dedup_override = Some(format!("once:initiative:{initiative}"));
        if is_new && let Some(owner) = i.owner_id {
            self.add(owner, "initiativeAddedAsOwner");
        }
        let mentioned = self.mention_users(&i.text).await?;
        self.add_all(mentioned, "initiativeMention");
        Ok(true)
    }

    /// Initiative-Statusbericht: Abonnenten und Erwähnungen.
    async fn initiative_update(
        &mut self,
        ev: &StoredEvent,
        update: Uuid,
        is_new: bool,
    ) -> AppResult<bool> {
        let Some(u) = self
            .tx
            .notifications()
            .initiative_update_facts(update)
            .await?
        else {
            return Ok(false);
        };
        self.fallback_actor(ev.origin, Some(u.author_id));
        self.target_initiative(u.initiative_id);
        self.refs.initiative_update_id = Some(u.id);
        self.dedup_override = Some(format!("once:initiativeUpdate:{}", u.id));
        let mentioned = self.mention_users(&u.body).await?;
        self.add_all(mentioned, "initiativeUpdateMention");
        if is_new {
            let subs = self
                .tx
                .notifications()
                .object_subscribers(
                    ObjectKind::Initiative,
                    u.initiative_id,
                    "initiativeUpdateCreated",
                )
                .await?;
            self.add_all(subs, "initiativeUpdateCreated");
        }
        Ok(true)
    }

    async fn sla(&mut self, issue: IssueId, kind: &'static str) -> AppResult<bool> {
        let Some(f) = self.tx.notifications().issue_facts(issue).await? else {
            return Ok(false);
        };
        self.target_issue(&f);
        let notifications = self.tx.notifications();
        let mut recipients = notifications.issue_subscribers(f.id).await?;
        recipients.extend(
            notifications
                .object_subscribers(ObjectKind::Team, f.team_id.0, kind)
                .await?,
        );
        recipients.extend(f.assignee_id);
        self.add_all(recipients, kind);
        Ok(true)
    }

    /// Bündelung (docs/05 §6.6): Ereignisse desselben Akteurs am selben Objekt innerhalb einer
    /// Minute gehen in die bestehende Notification des Empfängers ein (der spezifischere Typ
    /// bleibt, `bundled_types` listet alle). `true`, wenn das Ereignis erledigt ist (zusammengeführt
    /// oder bereits verarbeitet); die bestehende Notification meldet ein `NotificationUpdated`.
    /// Ohne menschlichen Akteur (Jobs, Automationen, Erinnerungen) wird nie gebündelt.
    async fn bundle(&mut self, n: &NewNotification) -> AppResult<bool> {
        let (Some(actor), Some(key)) = (n.actor_id, group_key_of(n)) else {
            return Ok(false);
        };
        if self
            .tx
            .notifications()
            .dedup_seen(n.user_id, &n.dedup_key)
            .await?
        {
            return Ok(true);
        }
        let since = self.now - Duration::seconds(BUNDLE_WINDOW_SECS);
        let Some(target) = self
            .tx
            .notifications()
            .find_bundle_target(n.user_id, &key, actor, since)
            .await?
        else {
            return Ok(false);
        };
        // Gegenläufige Ereignisse (zugewiesen/abgegeben, abonniert/abbestellt) ersetzen sich: der
        // Endzustand gewinnt (docs/05 §6.6).
        let wins = rank_of(&n.kind) < rank_of(&target.kind) || is_opposite(&n.kind, &target.kind);
        let mut types = if target.bundled_types.is_empty() {
            vec![target.kind.clone()]
        } else {
            target.bundled_types.clone()
        };
        if types.len() < MAX_BUNDLED_TYPES && !types.contains(&n.kind) {
            types.push(n.kind.clone());
        }
        let kind = if wins { n.kind.clone() } else { target.kind };
        let merge = BundleMerge {
            target: target.id,
            category: category_of(&kind).to_owned(),
            kind,
            take_refs: wins,
            bundled_types: types,
            // isPriority = any(...) (docs/05 §6.5): die Bündelung darf es nie zurücknehmen.
            is_priority: n.is_priority || target.is_priority,
        };
        self.tx
            .notifications()
            .merge_notification(n, &merge)
            .await?;
        self.tx
            .outbox()
            .push(NewEvent::new(
                DomainEvent::NotificationUpdated {
                    notification_id: target.id,
                    user_id: n.user_id,
                },
                EventOrigin::System,
            ))
            .await?;
        Ok(true)
    }

    /// Richtet die Fakten auf ein anderes Issue aus (nach dem Hauptereignis, z. B. `issueUnblocked`).
    fn retarget_issue(&mut self, f: &IssueFacts) {
        self.refs = Refs::default();
        self.cands.clear();
        self.target_issue(f);
        self.workspace_only = false;
    }

    /// Self-Suppression, Sichtbarkeit, Schreiben. Liefert die Zahl neuer Notifications.
    pub async fn deliver(
        &mut self,
        ids: &dyn crate::ports::IdGen,
        dedup_prefix: &str,
    ) -> AppResult<usize> {
        let cands: Vec<(UserId, &'static str)> = self
            .cands
            .iter()
            .filter(|(u, _)| Some(**u) != self.actor)
            .map(|(u, k)| (*u, *k))
            .collect();
        let users: Vec<UserId> = cands.iter().map(|(u, _)| *u).collect();
        let priority: HashMap<UserId, Vec<String>> = self
            .tx
            .notifications()
            .priority_categories(&users)
            .await?
            .into_iter()
            .collect();
        let teams = self.teams.clone();
        let dedup_prefix = self
            .dedup_override
            .clone()
            .unwrap_or_else(|| dedup_prefix.to_owned());
        let mut created = 0;
        for (user, kind) in cands {
            let visible = match (self.refs.issue_id, self.refs.team_id) {
                (Some(issue), Some(team)) => self.can_see_issue(user, issue, team).await?,
                _ => self.can_see_any(user, &teams).await?,
            };
            if !visible {
                continue;
            }
            let category = category_of(kind);
            let is_priority = is_always_priority(kind)
                || priority
                    .get(&user)
                    .is_some_and(|c| c.iter().any(|x| x == category));
            let n = NewNotification {
                id: NotificationId(ids.new_uuid()),
                user_id: user,
                kind: kind.to_owned(),
                category: category.to_owned(),
                target_type: self.refs.target_type.to_owned(),
                actor_id: self.actor,
                issue_id: self.refs.issue_id,
                team_id: self.refs.team_id,
                project_id: self.refs.project_id,
                project_update_id: self.refs.project_update_id,
                project_milestone_id: self.refs.project_milestone_id,
                initiative_id: self.refs.initiative_id,
                initiative_update_id: self.refs.initiative_update_id,
                document_id: self.refs.document_id,
                comment_id: self.refs.comment_id,
                parent_comment_id: self.refs.parent_comment_id,
                reaction_emoji: self.refs.reaction_emoji.clone(),
                is_priority,
                dedup_key: format!("{dedup_prefix}:{kind}"),
                now: self.now,
            };
            // Dasselbe Triage-Ereignis höchstens einmal je Empfänger (docs/05 §6.1 Regel 4): die
            // Triage-Verantwortung hat die Zeile schon angelegt.
            if kind == "issueAddedToTriage"
                && let Some(issue) = self.refs.issue_id
                && self
                    .tx
                    .notifications()
                    .has_issue_notification(user, issue, "triageResponsibilityIssueAddedToTriage")
                    .await?
            {
                continue;
            }
            if self.bundle(&n).await? {
                continue;
            }
            if self.tx.notifications().insert_notification(&n).await? {
                created += 1;
                self.tx
                    .outbox()
                    .push(NewEvent::new(
                        DomainEvent::NotificationCreated {
                            notification_id: n.id,
                            user_id: user,
                        },
                        EventOrigin::System,
                    ))
                    .await?;
            }
        }
        Ok(created)
    }
}

/// Ob das Event Notifications auslösen kann (vermeidet leere Transaktionen).
fn is_relevant(e: &DomainEvent) -> bool {
    matches!(
        e,
        DomainEvent::IssueCreated { .. }
            | DomainEvent::IssueUpdated { .. }
            | DomainEvent::CommentCreated { .. }
            | DomainEvent::CommentUpdated { .. }
            | DomainEvent::ReactionCreated { .. }
            | DomainEvent::IssueRelationCreated { .. }
            | DomainEvent::IssueSubscriptionCreated { .. }
            | DomainEvent::IssueSubscriptionRemoved { .. }
            | DomainEvent::ProjectUpdateCreated { .. }
            | DomainEvent::ProjectUpdateUpdated { .. }
            | DomainEvent::ProjectCreated { .. }
            | DomainEvent::ProjectUpdated { .. }
            | DomainEvent::ProjectMilestoneCreated { .. }
            | DomainEvent::ProjectMilestoneUpdated { .. }
            | DomainEvent::InitiativeCreated { .. }
            | DomainEvent::InitiativeUpdated { .. }
            | DomainEvent::InitiativeUpdateCreated { .. }
            | DomainEvent::InitiativeUpdateUpdated { .. }
            | DomainEvent::IssueSlaHighRisk { .. }
            | DomainEvent::IssueSlaBreached { .. }
    )
}

impl App {
    /// Verarbeitet ein Outbox-Event für den Konsumenten `notifications` (idempotent).
    /// Events aus dem Import erzeugen nichts. Liefert die Zahl neuer Notifications.
    pub async fn fan_out_event(&self, ev: &StoredEvent) -> AppResult<usize> {
        self.fan_out_event_with(ev, None).await
    }

    /// Wie [`App::fan_out_event`], zusätzlich mit der Quelle für View-Abos (WP31, siehe
    /// [`ViewSubscriptionSource`]).
    pub async fn fan_out_event_with(
        &self,
        ev: &StoredEvent,
        views: Option<&dyn ViewSubscriptionSource>,
    ) -> AppResult<usize> {
        if ev.origin == EventOrigin::Import || !is_relevant(&ev.event) {
            return Ok(0);
        }
        let mut tx = self.store.begin(ev.org_id).await?;
        let created = {
            let mut fan = Fan::new(&mut *tx, self.clock.now(), ev.actor_id)
                .await?
                .with_views(views);
            let mut created = if fan.plan(ev).await? {
                fan.deliver(self.ids.as_ref(), &format!("event:{}", ev.id))
                    .await?
            } else {
                0
            };
            // Issues, die durch dieses Ereignis frei wurden (`issueUnblocked`).
            for blocked in std::mem::take(&mut fan.unblocked) {
                let Some(f) = fan.tx.notifications().issue_facts(blocked).await? else {
                    continue;
                };
                fan.retarget_issue(&f);
                let mut recipients = fan.tx.notifications().issue_subscribers(f.id).await?;
                recipients.extend(f.assignee_id);
                fan.add_all(recipients, "issueUnblocked");
                created += fan
                    .deliver(
                        self.ids.as_ref(),
                        &format!("event:{}:unblocked:{}", ev.id, f.id),
                    )
                    .await?;
            }
            created
        };
        tx.commit().await?;
        Ok(created)
    }
}
