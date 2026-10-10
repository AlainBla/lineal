//! Notification-Fan-out (docs/10 §4.2, docs/05 §6): Outbox-Event → Empfänger → Notifications.
//!
//! Ablauf je Event: Abos nachführen (Creator, Assignee, Mention, Kommentar), Kandidaten sammeln
//! (der spezifischste Typ je Empfänger gewinnt), Self-Suppression, Sichtbarkeit je Empfänger
//! prüfen, Notifications idempotent schreiben (`dedup_key`).

use super::kinds::{category_of, is_always_priority, rank_of};
use crate::app::App;
use crate::context::Actor;
use crate::error::AppResult;
use crate::policy::{ActorView, TeamTree, compute_visible_team_ids};
use crate::ports::Tx;
use crate::ports::repos::notification::{
    IssueFacts, NewNotification, ObjectKind, SubscriptionReason,
};
use crate::ports::repos::{NewEvent, StoredEvent};
use chrono::{DateTime, Utc};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{CommentId, IssueId, NotificationId, ProjectId, TeamId, UserId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

const CLOSED: [&str; 3] = ["completed", "canceled", "duplicate"];

/// Bezugsobjekt einer Notification.
#[derive(Debug, Clone, Default)]
pub(crate) struct Refs {
    pub target_type: &'static str,
    pub issue_id: Option<IssueId>,
    pub team_id: Option<TeamId>,
    pub project_id: Option<ProjectId>,
    pub project_update_id: Option<Uuid>,
    pub comment_id: Option<CommentId>,
    pub parent_comment_id: Option<CommentId>,
    pub reaction_emoji: Option<String>,
}

/// Sammelt Kandidaten für genau ein Event bzw. eine Erinnerung.
pub(crate) struct Fan<'a> {
    tx: &'a mut dyn Tx,
    tree: TeamTree,
    visible: HashMap<UserId, Option<BTreeSet<TeamId>>>,
    now: DateTime<Utc>,
    pub refs: Refs,
    /// Sichtbar, wenn der Empfänger mindestens eines dieser Teams sieht (leer = immer).
    pub teams: Vec<TeamId>,
    pub actor: Option<UserId>,
    cands: BTreeMap<UserId, &'static str>,
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
            actor,
            cands: BTreeMap::new(),
        })
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
                    Some(compute_visible_team_ids(&view, &self.tree))
                }
                _ => None,
            };
            self.visible.insert(user, set);
        }
        Ok(match self.visible.get(&user) {
            Some(Some(set)) => teams.is_empty() || teams.iter().any(|t| set.contains(t)),
            _ => false,
        })
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
        Ok(())
    }

    /// Abo nachführen, aber nur für User, die das Issue sehen.
    async fn subscribe(
        &mut self,
        f: &IssueFacts,
        user: UserId,
        thread: Option<CommentId>,
        reason: SubscriptionReason,
    ) -> AppResult<()> {
        if self.can_see_any(user, &[f.team_id]).await? {
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
                ..
            } => {
                self.issue_updated(*issue_id, changed, mentioned_user_ids)
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
        Ok(true)
    }

    async fn issue_updated(
        &mut self,
        issue: IssueId,
        changed: &[String],
        mentioned: &[UserId],
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
        if has("priority")
            && f.priority == 1
            && let Some(a) = f.assignee_id
        {
            self.add(a, "issuePriorityUrgent");
        }
        if has("teamId") {
            // Abos von Usern, die das neue Team nicht sehen dürfen, entfallen (docs/05 §6.3).
            for s in self.tx.notifications().issue_subscribers(f.id).await? {
                if !self.can_see_any(s, &[f.team_id]).await? {
                    self.tx
                        .notifications()
                        .unsubscribe_issue(f.id, s, self.now)
                        .await?;
                }
            }
        }
        if has("stateId") {
            let closed = CLOSED.contains(&f.state_type.as_str());
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
        // Kommentare an Initiativen, Dokumenten, Meilensteinen und Berichten: noch nicht abgedeckt.
        Ok(false)
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
            return Ok(false);
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
        let mut created = 0;
        for (user, kind) in cands {
            if !self.can_see_any(user, &teams).await? {
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
                project_milestone_id: None,
                initiative_id: None,
                initiative_update_id: None,
                document_id: None,
                comment_id: self.refs.comment_id,
                parent_comment_id: self.refs.parent_comment_id,
                reaction_emoji: self.refs.reaction_emoji.clone(),
                is_priority,
                dedup_key: format!("{dedup_prefix}:{kind}"),
                now: self.now,
            };
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
            | DomainEvent::IssueSlaHighRisk { .. }
            | DomainEvent::IssueSlaBreached { .. }
    )
}

impl App {
    /// Verarbeitet ein Outbox-Event für den Konsumenten `notifications` (idempotent).
    /// Events aus dem Import erzeugen nichts. Liefert die Zahl neuer Notifications.
    pub async fn fan_out_event(&self, ev: &StoredEvent) -> AppResult<usize> {
        if ev.origin == EventOrigin::Import || !is_relevant(&ev.event) {
            return Ok(0);
        }
        let mut tx = self.store.begin(ev.org_id).await?;
        let created = {
            let mut fan = Fan::new(&mut *tx, self.clock.now(), ev.actor_id).await?;
            if fan.plan(ev).await? {
                fan.deliver(self.ids.as_ref(), &format!("event:{}", ev.id))
                    .await?
            } else {
                0
            }
        };
        tx.commit().await?;
        Ok(created)
    }
}
