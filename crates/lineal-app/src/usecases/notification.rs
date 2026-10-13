//! usecases – Inbox, Notifications, Abonnements und Einstellungen (WP08, docs/05 §6, docs/10 §4).
//!
//! Der Fan-out aus Outbox-Events steht in [`fanout`], periodische Aufgaben in [`jobs`].
//! Alle Inbox-Aktionen wirken nur auf **eigene** Notifications (fremde → `not_found`).

pub mod fanout;
pub mod jobs;
pub mod kinds;
pub mod views;

use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::notification::{
    BulkFilter, ChannelPreference, InboxQuery, IssueFacts, IssueSubscription, NotificationView,
    ObjectKind, ObjectSubscription, SubscribeOutcome, SubscriptionReason, UpsertObjectSubscription,
    UserNotificationPrefsPatch,
};
use crate::ports::repos::user::{Page, PageRequest};
use crate::resolve;
use chrono::{DateTime, Utc};
use kinds::{CATEGORIES, KNOWN_KINDS, channel_default, default_subscription_types, render};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{IssueId, NotificationId, TeamId, UserId};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Eine Notification mit gerendertem Text und Link.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationItem {
    pub view: NotificationView,
    pub title: String,
    pub subtitle: String,
    pub url: String,
}

/// Parameter von `list_notifications`.
#[derive(Debug, Clone, Default)]
pub struct ListNotificationsInput {
    pub unread_only: bool,
    pub include_snoozed: bool,
    pub include_archived: bool,
    pub page: PageRequest,
}

/// Parameter von `mark_notification`: mindestens einer ist nötig.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkNotificationInput {
    pub read: Option<bool>,
    /// `Some(None)` beendet das Snoozen, `Some(Some(t))` snoozt bis `t` (muss in der Zukunft liegen).
    pub snoozed_until_at: Option<Option<DateTime<Utc>>>,
}

/// Ergebnis von `subscribe_issue`/`unsubscribe_issue`.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueSubscriptionResult {
    pub issue_id: IssueId,
    pub user_id: UserId,
    /// `true`, wenn sich der Zustand geändert hat.
    pub changed: bool,
}

/// Ergebnis von `subscribe_document`/`unsubscribe_document`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSubscriptionResult {
    pub document_id: Uuid,
    /// Abo-Zustand nach der Aktion.
    pub subscribed: bool,
    /// `true`, wenn sich der Zustand geändert hat.
    pub changed: bool,
}

/// Parameter eines Objekt-Abos.
#[derive(Debug, Clone)]
pub struct SetObjectSubscriptionInput {
    pub kind: ObjectKind,
    pub target: Uuid,
    /// Nur bei `team`: `activeCycle`, `activeIssues`, `backlog`, `triage`, `upcomingCycle`.
    pub context_view_type: Option<String>,
    /// Default je Ziel und Kontext (docs/05 §6.3).
    pub types: Option<Vec<String>>,
    pub include_sub_initiative_updates: Option<bool>,
    pub active: Option<bool>,
}

/// Wirksame Einstellungen eines Users (docs/05 §6.9).
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationSettings {
    pub email_enabled: bool,
    /// `digest` oder `immediate`.
    pub email_format: String,
    pub email_only_priority: bool,
    pub email_hold_outside_work_hours: bool,
    pub email_immediate_urgent: bool,
    pub due_date_reminders: bool,
    pub priority_categories: Vec<String>,
    /// Wirksamer Zustand aller Kategorien für `email`, `desktop` und `mobile`.
    pub channels: Vec<ChannelPreference>,
}

/// Änderung der Einstellungen (nur gesetzte Felder).
#[derive(Debug, Clone, Default)]
pub struct NotificationSettingsPatch {
    pub email_enabled: Option<bool>,
    pub email_format: Option<String>,
    pub email_only_priority: Option<bool>,
    pub email_hold_outside_work_hours: Option<bool>,
    pub email_immediate_urgent: Option<bool>,
    pub due_date_reminders: Option<bool>,
    pub priority_categories: Option<Vec<String>>,
    pub channels: Vec<ChannelPreference>,
}

const CONTEXTS: [&str; 5] = [
    "activeCycle",
    "activeIssues",
    "backlog",
    "triage",
    "upcomingCycle",
];
const CHANNELS: [&str; 3] = ["email", "desktop", "mobile"];

/// Der handelnde User (Notifications gehören immer einem User; System/Apps ohne User nicht).
/// Schreibende Inbox-/Abo-Aktionen brauchen Scope `write` (auch für eigene Daten).
fn require_write(ctx: &RequestCtx) -> AppResult<()> {
    if ctx.scopes.write {
        Ok(())
    } else {
        Err(
            AppError::forbidden("Token scope does not allow write operations.").with_details(
                serde_json::json!({ "reason": "insufficient_scope", "scope": "write" }),
            ),
        )
    }
}

fn me(ctx: &RequestCtx) -> AppResult<UserId> {
    match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => Ok(id),
        Actor::System => Err(AppError::forbidden(
            "Notifications are only available to user contexts.",
        )),
    }
}

fn not_found() -> AppError {
    AppError::not_found("Notification not found.")
}

/// Eigene Notification, deren Bezugsobjekt noch sichtbar ist (sonst `not_found`).
async fn fetch_visible(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    user: UserId,
    id: NotificationId,
) -> AppResult<NotificationView> {
    let view = tx
        .notifications()
        .get_notification(user, id)
        .await?
        .ok_or_else(not_found)?;
    let visible = snap.visible_team_ids();
    let ok = match (&view.issue_team_id, view.n.project_id) {
        (Some(team), _) => {
            visible.contains(team)
                || match view.n.issue_id {
                    Some(issue) => tx.documents().issue_shared_with(issue, user).await?,
                    None => false,
                }
        }
        (None, Some(project)) if view.n.target_type == "project" => tx
            .notifications()
            .project_team_ids(project)
            .await?
            .iter()
            .any(|t| visible.contains(t)),
        _ => true,
    };
    if ok { Ok(view) } else { Err(not_found()) }
}

fn issue_not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Issue \"{reference}\" not found."))
}

async fn org_base(app: &App, tx: &mut dyn Tx) -> AppResult<String> {
    let base = app.config.base_url.trim_end_matches('/');
    Ok(match tx.orgs().get().await? {
        Some(o) => format!("{base}/{}", o.url_key),
        None => base.to_owned(),
    })
}

/// Schreibt ein persönliches Inbox-Event (Sync an den Besitzer, WP15) in die Outbox.
async fn push_notification_event(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    event: DomainEvent,
) -> AppResult<()> {
    tx.outbox()
        .push(NewEvent::from_ctx(event, ctx))
        .await
        .map(|_| ())
}

fn to_item(view: NotificationView, base: &str) -> NotificationItem {
    let r = render(&view, base);
    NotificationItem {
        view,
        title: r.title,
        subtitle: r.subtitle,
        url: r.url,
    }
}

/// Parser für den Filter-Namen (`all`, `read`, `completed`).
pub fn parse_bulk(filter: Option<&str>) -> AppResult<BulkFilter> {
    match filter {
        None | Some("all") => Ok(BulkFilter::All),
        Some("read") => Ok(BulkFilter::Read),
        Some("completed") => Ok(BulkFilter::Completed),
        Some(other) => Err(AppError::invalid_request(format!(
            "unknown filter `{other}` (expected all, read or completed)"
        ))),
    }
}

impl App {
    // --- Inbox ---

    /// Eigene Notifications, neueste zuerst (docs/05 §6.5; MCP `get_notifications`).
    pub async fn list_notifications(
        &self,
        ctx: &RequestCtx,
        input: ListNotificationsInput,
    ) -> AppResult<Page<NotificationItem>> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let query = InboxQuery {
            unread_only: input.unread_only,
            include_snoozed: input.include_snoozed,
            include_archived: input.include_archived,
            visible_teams: snap.visible_team_ids().into_iter().collect(),
        };
        let page = tx
            .notifications()
            .list_notifications(user, &query, self.clock.now(), &input.page)
            .await?;
        let base = org_base(self, &mut *tx).await?;
        Ok(Page {
            items: page.items.into_iter().map(|v| to_item(v, &base)).collect(),
            next: page.next,
        })
    }

    /// Eine eigene Notification.
    pub async fn get_notification(
        &self,
        ctx: &RequestCtx,
        id: NotificationId,
    ) -> AppResult<NotificationItem> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        let base = org_base(self, &mut *tx).await?;
        Ok(to_item(view, &base))
    }

    /// Anzahl ungelesener Gruppen (Seitenleiste).
    pub async fn unread_notification_count(&self, ctx: &RequestCtx) -> AppResult<i64> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let visible: Vec<TeamId> = snap.visible_team_ids().into_iter().collect();
        tx.notifications()
            .count_unread_groups(user, &visible, self.clock.now())
            .await
    }

    /// `mark_notification`: gelesen/ungelesen (einzeln) und Snooze/Unsnooze (ganze Gruppe).
    pub async fn mark_notification(
        &self,
        ctx: &RequestCtx,
        id: NotificationId,
        input: MarkNotificationInput,
    ) -> AppResult<NotificationItem> {
        require_write(ctx)?;
        let user = me(ctx)?;
        if input.read.is_none() && input.snoozed_until_at.is_none() {
            return Err(AppError::invalid_request(
                "At least one of `read` and `snoozedUntilAt` is required.",
            ));
        }
        let now = self.clock.now();
        if let Some(Some(until)) = input.snoozed_until_at
            && until <= now
        {
            return Err(
                AppError::invalid_request("`snoozedUntilAt` must be in the future.")
                    .with_details(json!({ "path": "/snoozedUntilAt" })),
            );
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        if let Some(until) = input.snoozed_until_at {
            tx.notifications()
                .snooze_group(user, &view.n.group_key, until, now)
                .await?;
        }
        // Gelesen/ungelesen gilt für die ganze Gruppe (docs/05 §6.7/6.8).
        let mut changed: Vec<NotificationId> = vec![id];
        if let Some(read) = input.read {
            changed.extend(
                tx.notifications()
                    .set_read(user, id, read, now)
                    .await?
                    .into_iter()
                    .filter(|c| *c != id),
            );
        }
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        let base = org_base(self, &mut *tx).await?;
        for notification_id in changed {
            push_notification_event(
                &mut *tx,
                ctx,
                DomainEvent::NotificationUpdated {
                    notification_id,
                    user_id: user,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(to_item(view, &base))
    }

    /// Öffnen einer Notification: alle ungelesenen der Gruppe werden gelesen (docs/05 §6.4 Nr. 1).
    pub async fn open_notification(
        &self,
        ctx: &RequestCtx,
        id: NotificationId,
    ) -> AppResult<NotificationItem> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        tx.notifications()
            .mark_group_read(user, &view.n.group_key, self.clock.now())
            .await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        let base = org_base(self, &mut *tx).await?;
        push_notification_event(
            &mut *tx,
            ctx,
            DomainEvent::NotificationUpdated {
                notification_id: id,
                user_id: user,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(to_item(view, &base))
    }

    /// Alle (ggf. gefilterten) Notifications als gelesen markieren (`notificationMarkReadAll`).
    pub async fn mark_all_notifications_read(
        &self,
        ctx: &RequestCtx,
        filter: Option<&str>,
    ) -> AppResult<u64> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let filter = parse_bulk(filter)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let visible: Vec<TeamId> = snap.visible_team_ids().into_iter().collect();
        let ids = tx
            .notifications()
            .mark_all_read(user, &visible, filter, self.clock.now())
            .await?;
        // Jede geänderte Notification geht als Sync-Aktion an die Clients des Users.
        for id in &ids {
            push_notification_event(
                &mut *tx,
                ctx,
                DomainEvent::NotificationUpdated {
                    notification_id: *id,
                    user_id: user,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as u64)
    }

    /// Archiviert die Gruppe der Notification (UI „Delete“); `archive = false` ist „Undo“.
    pub async fn archive_notification(
        &self,
        ctx: &RequestCtx,
        id: NotificationId,
        archive: bool,
    ) -> AppResult<NotificationItem> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        tx.notifications()
            .set_group_archived(user, &view.n.group_key, archive, self.clock.now())
            .await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let view = fetch_visible(&mut *tx, &snap, user, id).await?;
        let base = org_base(self, &mut *tx).await?;
        push_notification_event(
            &mut *tx,
            ctx,
            DomainEvent::NotificationArchived {
                notification_id: id,
                user_id: user,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(to_item(view, &base))
    }

    /// `notificationArchiveAll(filter)`: `all`, `read` oder `completed`. Liefert die Anzahl.
    pub async fn archive_all_notifications(
        &self,
        ctx: &RequestCtx,
        filter: Option<&str>,
    ) -> AppResult<u64> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let filter = parse_bulk(filter)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let visible: Vec<TeamId> = snap.visible_team_ids().into_iter().collect();
        let ids = tx
            .notifications()
            .archive_all(user, &visible, filter, self.clock.now())
            .await?;
        for id in &ids {
            push_notification_event(
                &mut *tx,
                ctx,
                DomainEvent::NotificationArchived {
                    notification_id: *id,
                    user_id: user,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as u64)
    }

    // --- Issue-Abos ---

    async fn issue_for_subscription(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        reference: &str,
        action: Action,
    ) -> AppResult<IssueFacts> {
        let facts = tx
            .notifications()
            .issue_facts_by_ref(reference)
            .await?
            .ok_or_else(|| issue_not_found(reference))?;
        // Private Teams: nicht sichtbar → not_found, nie forbidden.
        snap.require_team(ctx, action, facts.team_id).map_err(|e| {
            if e.kind == crate::error::ErrorKind::NotFound {
                issue_not_found(reference)
            } else {
                e
            }
        })?;
        Ok(facts)
    }

    async fn subscription_target(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        user_ref: Option<&str>,
    ) -> AppResult<UserId> {
        match user_ref {
            None => me(ctx),
            Some(r) => Ok(resolve::user::resolve_in(tx, snap, ctx, r).await?.id),
        }
    }

    /// „Subscribe“ (`manual`) bzw. „Change subscribers“ (`addedByOther`, `user` ≠ Aufrufer).
    pub async fn subscribe_issue(
        &self,
        ctx: &RequestCtx,
        issue: &str,
        user: Option<&str>,
    ) -> AppResult<IssueSubscriptionResult> {
        require_write(ctx)?;
        let caller = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let target = self.subscription_target(&mut *tx, &snap, ctx, user).await?;
        let action = if target == caller {
            Action::ViewTeam
        } else {
            Action::UpdateIssue
        };
        let facts = self
            .issue_for_subscription(&mut *tx, &snap, ctx, issue, action)
            .await?;
        if target != caller {
            // Der Hinzugefügte muss das Issue sehen dürfen (docs/05 §6.1 Nr. 3).
            let mut fan = fanout::Fan::new(&mut *tx, self.clock.now(), None).await?;
            if !fan.can_see_any(target, &[facts.team_id]).await? {
                return Err(AppError::invalid_request(
                    "The user cannot access this issue.",
                ));
            }
        }
        let reason = if target == caller {
            SubscriptionReason::Manual
        } else {
            SubscriptionReason::AddedByOther
        };
        let outcome = tx
            .notifications()
            .subscribe_issue(facts.id, target, None, reason, self.clock.now())
            .await?;
        let changed = outcome != SubscribeOutcome::Unchanged;
        if changed {
            tx.outbox()
                .push(NewEvent::from_ctx(
                    DomainEvent::IssueSubscriptionCreated {
                        issue_id: facts.id,
                        user_id: target,
                    },
                    ctx,
                ))
                .await?;
        }
        tx.commit().await?;
        Ok(IssueSubscriptionResult {
            issue_id: facts.id,
            user_id: target,
            changed,
        })
    }

    /// „Unsubscribe“: setzt das Abo (inkl. Thread-Abos) auf `active = false`.
    pub async fn unsubscribe_issue(
        &self,
        ctx: &RequestCtx,
        issue: &str,
        user: Option<&str>,
    ) -> AppResult<IssueSubscriptionResult> {
        require_write(ctx)?;
        let caller = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let target = self.subscription_target(&mut *tx, &snap, ctx, user).await?;
        let action = if target == caller {
            Action::ViewTeam
        } else {
            Action::UpdateIssue
        };
        let facts = self
            .issue_for_subscription(&mut *tx, &snap, ctx, issue, action)
            .await?;
        let changed = tx
            .notifications()
            .unsubscribe_issue(facts.id, target, self.clock.now())
            .await?;
        if changed {
            tx.outbox()
                .push(NewEvent::from_ctx(
                    DomainEvent::IssueSubscriptionRemoved {
                        issue_id: facts.id,
                        user_id: target,
                    },
                    ctx,
                ))
                .await?;
        }
        tx.commit().await?;
        Ok(IssueSubscriptionResult {
            issue_id: facts.id,
            user_id: target,
            changed,
        })
    }

    /// Lädt das Dokument für ein Abo und prüft die Sichtbarkeit (Gäste nur bei Teambezug,
    /// sonst `not_found`).
    async fn document_for_subscription(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        document: Uuid,
    ) -> AppResult<Uuid> {
        let missing = || AppError::not_found("Document not found.");
        let facts = tx
            .notifications()
            .document_facts(document)
            .await?
            .ok_or_else(missing)?;
        let visible = snap.visible_team_ids();
        let ok = if facts.workspace_only {
            snap.actor.role != lineal_domain::enums::UserRole::Guest
        } else {
            facts.teams.iter().any(|t| visible.contains(t))
        };
        if facts.trashed || !ok {
            return Err(missing());
        }
        Ok(facts.id)
    }

    /// Dokument abonnieren (`manual`, docs/05 §6.3); reaktiviert ein beendetes Abo.
    pub async fn subscribe_document(
        &self,
        ctx: &RequestCtx,
        document: Uuid,
    ) -> AppResult<DocumentSubscriptionResult> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let id = self
            .document_for_subscription(&mut *tx, &snap, document)
            .await?;
        let outcome = tx
            .notifications()
            .subscribe_document(id, user, SubscriptionReason::Manual, self.clock.now())
            .await?;
        tx.commit().await?;
        Ok(DocumentSubscriptionResult {
            document_id: id,
            subscribed: true,
            changed: outcome != SubscribeOutcome::Unchanged,
        })
    }

    /// Dokument-Abo beenden (nur das eigene).
    pub async fn unsubscribe_document(
        &self,
        ctx: &RequestCtx,
        document: Uuid,
    ) -> AppResult<DocumentSubscriptionResult> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let id = self
            .document_for_subscription(&mut *tx, &snap, document)
            .await?;
        let changed = tx
            .notifications()
            .unsubscribe_document(id, user, self.clock.now())
            .await?;
        tx.commit().await?;
        Ok(DocumentSubscriptionResult {
            document_id: id,
            subscribed: false,
            changed,
        })
    }

    /// Ob der Aufrufer das Dokument abonniert hat.
    pub async fn is_document_subscribed(
        &self,
        ctx: &RequestCtx,
        document: Uuid,
    ) -> AppResult<bool> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let id = self
            .document_for_subscription(&mut *tx, &snap, document)
            .await?;
        tx.notifications().is_document_subscriber(id, user).await
    }

    /// Abo-Datensätze eines Issues (aktive und beendete).
    pub async fn list_issue_subscriptions(
        &self,
        ctx: &RequestCtx,
        issue: &str,
    ) -> AppResult<Vec<IssueSubscription>> {
        me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let facts = self
            .issue_for_subscription(&mut *tx, &snap, ctx, issue, Action::ViewTeam)
            .await?;
        tx.notifications().list_issue_subscriptions(facts.id).await
    }

    // --- Objekt-Abos ---

    /// Legt ein Objekt-Abo (Team, Projekt, Initiative, Cycle, Label) an oder ändert es.
    pub async fn set_notification_subscription(
        &self,
        ctx: &RequestCtx,
        input: SetObjectSubscriptionInput,
    ) -> AppResult<ObjectSubscription> {
        require_write(ctx)?;
        let user = me(ctx)?;
        if let Some(c) = &input.context_view_type {
            if input.kind != ObjectKind::Team || !CONTEXTS.contains(&c.as_str()) {
                return Err(AppError::invalid_request(
                    "`contextViewType` is only valid for team subscriptions.",
                )
                .with_details(json!({ "field": "contextViewType" })));
            }
        }
        if let Some(types) = &input.types {
            let ok = !types.is_empty() && types.iter().all(|t| KNOWN_KINDS.contains(&t.as_str()));
            if !ok {
                return Err(AppError::invalid_request(
                    "`notificationSubscriptionTypes` must be a non-empty list of known notification types.",
                )
                .with_details(json!({ "field": "notificationSubscriptionTypes" })));
            }
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let visible: BTreeSet<TeamId> = snap.visible_team_ids();
        let missing = || AppError::not_found("Subscription target not found.");
        match input.kind {
            ObjectKind::Team => {
                snap.require_team(ctx, Action::ViewTeam, TeamId(input.target))?;
            }
            ObjectKind::Project => {
                let teams = tx
                    .notifications()
                    .project_team_ids(lineal_domain::ids::ProjectId(input.target))
                    .await?;
                if !teams.iter().any(|t| visible.contains(t)) {
                    return Err(missing());
                }
            }
            ObjectKind::Cycle => {
                let team = tx
                    .notifications()
                    .cycle_team(input.target)
                    .await?
                    .ok_or_else(missing)?;
                snap.require_team(ctx, Action::ViewTeam, team)?;
            }
            ObjectKind::Label => match tx.notifications().label_team(input.target).await? {
                None => return Err(missing()),
                Some(Some(team)) => {
                    snap.require_team(ctx, Action::ViewTeam, team)?;
                }
                Some(None) => {}
            },
            ObjectKind::Initiative => {
                snap.require_workspace(ctx, Action::ViewInitiatives)?;
                if !tx.notifications().initiative_exists(input.target).await? {
                    return Err(missing());
                }
            }
        }
        let types = input.types.clone().unwrap_or_else(|| {
            default_subscription_types(input.kind, input.context_view_type.as_deref())
        });
        let sub = tx
            .notifications()
            .upsert_object_subscription(&UpsertObjectSubscription {
                id: self.ids.new_uuid(),
                subscriber_id: user,
                kind: input.kind,
                target_id: input.target,
                context_view_type: input.context_view_type,
                types,
                include_sub_initiative_updates: input
                    .include_sub_initiative_updates
                    .unwrap_or(false),
                active: input.active.unwrap_or(true),
                now: self.clock.now(),
            })
            .await?;
        tx.commit().await?;
        Ok(sub)
    }

    /// Eigene Objekt-Abos.
    pub async fn list_notification_subscriptions(
        &self,
        ctx: &RequestCtx,
    ) -> AppResult<Vec<ObjectSubscription>> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.notifications().list_object_subscriptions(user).await
    }

    // --- Einstellungen ---

    /// Eigene Einstellungen; fehlende Kanal-Zeilen werden mit den Defaults aufgefüllt.
    pub async fn get_notification_settings(
        &self,
        ctx: &RequestCtx,
    ) -> AppResult<NotificationSettings> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        settings_in(&mut *tx, user).await
    }

    /// Ändert die eigenen Einstellungen. Die Inbox ist nicht abschaltbar (nur `email`,
    /// `desktop`, `mobile`).
    pub async fn update_notification_settings(
        &self,
        ctx: &RequestCtx,
        patch: NotificationSettingsPatch,
    ) -> AppResult<NotificationSettings> {
        require_write(ctx)?;
        let user = me(ctx)?;
        if let Some(f) = &patch.email_format
            && !matches!(f.as_str(), "digest" | "immediate")
        {
            return Err(
                AppError::invalid_request("`emailFormat` must be digest or immediate.")
                    .with_details(json!({ "field": "emailFormat" })),
            );
        }
        let mut patch = patch;
        if let Some(cats) = &mut patch.priority_categories {
            if cats.iter().any(|c| !CATEGORIES.contains(&c.as_str())) {
                return Err(AppError::invalid_request("unknown notification category")
                    .with_details(json!({ "field": "priorityCategories" })));
            }
            // Duplikate entfallen, die Reihenfolge bleibt (docs/05 §6.9).
            let mut seen = BTreeSet::new();
            cats.retain(|c| seen.insert(c.clone()));
        }
        for c in &patch.channels {
            if !CATEGORIES.contains(&c.category.as_str()) || !CHANNELS.contains(&c.channel.as_str())
            {
                return Err(AppError::invalid_request(
                    "channel preferences need a known category and a channel of email, desktop or mobile \
                     (the inbox cannot be disabled)",
                )
                .with_details(json!({ "field": "channels" })));
            }
        }
        let mut extra = serde_json::Map::new();
        for (key, v) in [
            ("emailEnabled", patch.email_enabled),
            ("emailOnlyPriority", patch.email_only_priority),
            (
                "emailHoldOutsideWorkHours",
                patch.email_hold_outside_work_hours,
            ),
            ("emailImmediateUrgent", patch.email_immediate_urgent),
        ] {
            if let Some(b) = v {
                extra.insert(key.to_owned(), Value::Bool(b));
            }
        }
        let now = self.clock.now();
        let mut tx = self.store.begin(ctx.org_id).await?;
        // `emailOnlyPriority` braucht mindestens eine Priority-Kategorie (Ergebnis nach dem Patch).
        let current = settings_in(&mut *tx, user).await?;
        let only_priority = patch
            .email_only_priority
            .unwrap_or(current.email_only_priority);
        let categories = patch
            .priority_categories
            .as_ref()
            .unwrap_or(&current.priority_categories);
        if only_priority && categories.is_empty() {
            return Err(AppError::invalid_request(
                "`emailOnlyPriority` requires at least one priority category.",
            )
            .with_details(json!({ "field": "emailOnlyPriority" })));
        }
        tx.notifications()
            .update_prefs(
                user,
                &UserNotificationPrefsPatch {
                    email_format: patch.email_format,
                    due_date_reminders: patch.due_date_reminders,
                    priority_categories: patch.priority_categories,
                    settings: (!extra.is_empty()).then_some(Value::Object(extra)),
                },
                now,
            )
            .await?;
        for c in &patch.channels {
            tx.notifications()
                .set_channel_preference(user, c, now)
                .await?;
        }
        let out = settings_in(&mut *tx, user).await?;
        tx.commit().await?;
        Ok(out)
    }
}

async fn settings_in(tx: &mut dyn Tx, user: UserId) -> AppResult<NotificationSettings> {
    let prefs = tx
        .notifications()
        .get_prefs(user)
        .await?
        .ok_or_else(|| AppError::not_found("User not found."))?;
    let stored = tx.notifications().list_channel_preferences(user).await?;
    let mut channels = Vec::new();
    for category in CATEGORIES {
        for channel in CHANNELS {
            let enabled = stored
                .iter()
                .find(|p| p.category == category && p.channel == channel)
                .map_or_else(|| channel_default(category, channel), |p| p.enabled);
            channels.push(ChannelPreference {
                category: category.to_owned(),
                channel: channel.to_owned(),
                enabled,
            });
        }
    }
    let flag = |k: &str, d: bool| jobs::setting(&prefs, k, d);
    Ok(NotificationSettings {
        email_enabled: flag("emailEnabled", true),
        email_only_priority: flag("emailOnlyPriority", false),
        email_hold_outside_work_hours: flag("emailHoldOutsideWorkHours", true),
        email_immediate_urgent: flag("emailImmediateUrgent", true),
        email_format: prefs.email_format,
        due_date_reminders: prefs.due_date_reminders,
        priority_categories: prefs.priority_categories,
        channels,
    })
}
