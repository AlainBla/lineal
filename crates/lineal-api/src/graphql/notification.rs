//! GraphQL-Teilobjekte des Bereichs `notification` (WP08): Inbox, Abos, Einstellungen.
//!
//! Die Platzhalterfelder `_notificationReady`/`_notificationMutationReady` bleiben bestehen, weil
//! ein Test in `graphql/mod.rs` (gemeinsame Datei) sie im SDL erwartet.

use crate::context::{app, request_ctx};
use crate::error::GqlResultExt;
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::notification::{ChannelPreference, ObjectKind};
use lineal_app::ports::repos::user::{OrderBy, PageCursor, PageRequest};
use lineal_app::usecases::notification::{
    ListNotificationsInput, MarkNotificationInput, NotificationItem, NotificationSettings,
    NotificationSettingsPatch, SetObjectSubscriptionInput,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::NotificationId;
use std::str::FromStr;
use uuid::Uuid;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `Notification`.
#[derive(SimpleObject)]
#[graphql(name = "Notification")]
pub struct NotificationGql {
    pub id: String,
    #[graphql(name = "type")]
    pub kind: String,
    pub category: String,
    pub title: String,
    pub subtitle: String,
    pub url: String,
    pub group_key: String,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub issue_id: Option<String>,
    pub issue_identifier: Option<String>,
    pub project_id: Option<String>,
    pub initiative_id: Option<String>,
    pub document_id: Option<String>,
    pub comment_id: Option<String>,
    pub is_priority: bool,
    pub read_at: Option<String>,
    pub snoozed_until_at: Option<String>,
    pub unsnoozed_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<NotificationItem> for NotificationGql {
    fn from(i: NotificationItem) -> Self {
        let v = i.view;
        let n = v.n;
        Self {
            id: n.id.to_string(),
            kind: n.kind,
            category: n.category,
            title: i.title,
            subtitle: i.subtitle,
            url: i.url,
            group_key: n.group_key,
            actor_id: n.actor_id.map(|a| a.to_string()),
            actor_name: v.actor_name,
            issue_id: n.issue_id.map(|a| a.to_string()),
            issue_identifier: v.issue_identifier,
            project_id: n.project_id.map(|a| a.to_string()),
            initiative_id: n.initiative_id.map(|a| a.to_string()),
            document_id: n.document_id.map(|a| a.to_string()),
            comment_id: n.comment_id.map(|a| a.to_string()),
            is_priority: n.is_priority,
            read_at: n.read_at.map(ts),
            snoozed_until_at: n.snoozed_until_at.map(ts),
            unsnoozed_at: n.unsnoozed_at.map(ts),
            archived_at: n.archived_at.map(ts),
            created_at: ts(n.created_at),
            updated_at: ts(n.updated_at),
        }
    }
}

#[derive(SimpleObject)]
pub struct NotificationPageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "NotificationConnection")]
pub struct NotificationConnection {
    pub nodes: Vec<NotificationGql>,
    pub page_info: NotificationPageInfo,
}

#[derive(SimpleObject)]
#[graphql(name = "NotificationPayload")]
pub struct NotificationPayload {
    pub success: bool,
    pub notification: Option<NotificationGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "NotificationBatchActionPayload")]
pub struct NotificationBatchActionPayload {
    pub success: bool,
    pub count: i64,
}

#[derive(SimpleObject)]
#[graphql(name = "IssueSubscriptionPayload")]
pub struct IssueSubscriptionPayload {
    pub success: bool,
    pub issue_id: String,
    pub user_id: String,
    pub changed: bool,
}

#[derive(SimpleObject, Clone)]
#[graphql(name = "NotificationChannelSetting")]
pub struct ChannelSettingGql {
    pub category: String,
    pub channel: String,
    pub enabled: bool,
}

#[derive(SimpleObject)]
#[graphql(name = "NotificationSettings")]
pub struct NotificationSettingsGql {
    pub email_enabled: bool,
    pub email_format: String,
    pub email_only_priority: bool,
    pub email_hold_outside_work_hours: bool,
    pub email_immediate_urgent: bool,
    pub due_date_reminders: bool,
    pub priority_categories: Vec<String>,
    pub channels: Vec<ChannelSettingGql>,
}

impl From<NotificationSettings> for NotificationSettingsGql {
    fn from(s: NotificationSettings) -> Self {
        Self {
            email_enabled: s.email_enabled,
            email_format: s.email_format,
            email_only_priority: s.email_only_priority,
            email_hold_outside_work_hours: s.email_hold_outside_work_hours,
            email_immediate_urgent: s.email_immediate_urgent,
            due_date_reminders: s.due_date_reminders,
            priority_categories: s.priority_categories,
            channels: s
                .channels
                .into_iter()
                .map(|c| ChannelSettingGql {
                    category: c.category,
                    channel: c.channel,
                    enabled: c.enabled,
                })
                .collect(),
        }
    }
}

#[derive(InputObject)]
pub struct NotificationUpdateInput {
    /// `true` = gelesen, `false` = ungelesen.
    pub read: Option<bool>,
    /// Zeitpunkt (ISO-8601) zum Snoozen.
    pub snoozed_until_at: Option<String>,
    /// `true` beendet das Snoozen.
    pub unsnooze: Option<bool>,
}

#[derive(InputObject)]
pub struct ChannelSettingInput {
    pub category: String,
    pub channel: String,
    pub enabled: bool,
}

#[derive(InputObject)]
pub struct NotificationSettingsInput {
    pub email_enabled: Option<bool>,
    pub email_format: Option<String>,
    pub email_only_priority: Option<bool>,
    pub email_hold_outside_work_hours: Option<bool>,
    pub email_immediate_urgent: Option<bool>,
    pub due_date_reminders: Option<bool>,
    pub priority_categories: Option<Vec<String>>,
    pub channels: Option<Vec<ChannelSettingInput>>,
}

#[derive(InputObject)]
pub struct NotificationSubscriptionInput {
    /// `team`, `project`, `initiative`, `cycle` oder `label`.
    pub target_type: String,
    pub target_id: String,
    pub context_view_type: Option<String>,
    pub notification_subscription_types: Option<Vec<String>>,
    pub include_sub_initiative_updates: Option<bool>,
    pub active: Option<bool>,
}

#[derive(SimpleObject)]
#[graphql(name = "NotificationSubscription")]
pub struct NotificationSubscriptionGql {
    pub id: String,
    pub target_type: String,
    pub target_id: String,
    pub context_view_type: Option<String>,
    pub notification_subscription_types: Vec<String>,
    pub include_sub_initiative_updates: bool,
    pub active: bool,
}

fn invalid(msg: &str) -> AppError {
    AppError::invalid_request(msg.to_owned())
}

fn parse_id(s: &str) -> AppResult<NotificationId> {
    NotificationId::from_str(s.trim()).map_err(|_| AppError::not_found("Notification not found."))
}

fn parse_kind(s: &str) -> AppResult<ObjectKind> {
    Ok(match s {
        "team" => ObjectKind::Team,
        "project" => ObjectKind::Project,
        "initiative" => ObjectKind::Initiative,
        "cycle" => ObjectKind::Cycle,
        "label" => ObjectKind::Label,
        _ => {
            return Err(invalid(
                "`targetType` must be team, project, initiative, cycle or label",
            ));
        }
    })
}

#[derive(Default)]
pub struct NotificationQuery;

#[Object]
impl NotificationQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_notificationReady")]
    async fn notification_ready(&self) -> bool {
        true
    }

    /// Eigene Notifications, neueste zuerst (ausgeblendet: archivierte, gesnoozte, Objekte ohne Zugriff).
    async fn notifications(
        &self,
        ctx: &Context<'_>,
        unread_only: Option<bool>,
        include_snoozed: Option<bool>,
        include_archived: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<NotificationConnection> {
        let limit = i64::from(first.unwrap_or(50).clamp(1, 250));
        let cursor = match after {
            Some(c) => {
                let c = crate::pagination::Cursor::decode(
                    &c,
                    crate::pagination::PaginationOrderBy::CreatedAt,
                )
                .gql(ctx)?;
                Some(PageCursor {
                    key: DateTime::parse_from_rfc3339(&c.sort_key)
                        .map_err(|_| async_graphql::Error::new("Invalid cursor."))?
                        .with_timezone(&Utc),
                    id: c.id,
                })
            }
            None => None,
        };
        let page = app(ctx)?
            .list_notifications(
                request_ctx(ctx)?,
                ListNotificationsInput {
                    unread_only: unread_only.unwrap_or(false),
                    include_snoozed: include_snoozed.unwrap_or(false),
                    include_archived: include_archived.unwrap_or(false),
                    page: PageRequest {
                        limit,
                        cursor,
                        order_by: OrderBy::CreatedAt,
                    },
                },
            )
            .await
            .gql(ctx)?;
        let end_cursor = page.items.last().map(|i| {
            crate::pagination::Cursor {
                sort_key: i
                    .view
                    .n
                    .created_at
                    .to_rfc3339_opts(SecondsFormat::Micros, true),
                id: i.view.n.id.0,
            }
            .encode(crate::pagination::PaginationOrderBy::CreatedAt)
        });
        Ok(NotificationConnection {
            page_info: NotificationPageInfo {
                has_next_page: page.next.is_some(),
                end_cursor,
            },
            nodes: page.items.into_iter().map(Into::into).collect(),
        })
    }

    /// Eine eigene Notification.
    async fn notification(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<NotificationGql> {
        let id = parse_id(&id).gql(ctx)?;
        Ok(app(ctx)?
            .get_notification(request_ctx(ctx)?, id)
            .await
            .gql(ctx)?
            .into())
    }

    /// Anzahl ungelesener Gruppen (Seitenleiste).
    async fn notifications_unread_count(&self, ctx: &Context<'_>) -> async_graphql::Result<i64> {
        app(ctx)?
            .unread_notification_count(request_ctx(ctx)?)
            .await
            .gql(ctx)
    }

    /// Eigene Benachrichtigungs-Einstellungen.
    async fn notification_settings(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<NotificationSettingsGql> {
        Ok(app(ctx)?
            .get_notification_settings(request_ctx(ctx)?)
            .await
            .gql(ctx)?
            .into())
    }

    /// Eigene Objekt-Abos (Team, Projekt, Initiative, Cycle, Label).
    async fn notification_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<NotificationSubscriptionGql>> {
        let subs = app(ctx)?
            .list_notification_subscriptions(request_ctx(ctx)?)
            .await
            .gql(ctx)?;
        Ok(subs.into_iter().map(sub_gql).collect())
    }
}

fn sub_gql(
    s: lineal_app::ports::repos::notification::ObjectSubscription,
) -> NotificationSubscriptionGql {
    NotificationSubscriptionGql {
        id: s.id.to_string(),
        target_type: s.kind.as_str().to_owned(),
        target_id: s.target_id.to_string(),
        context_view_type: s.context_view_type,
        notification_subscription_types: s.types,
        include_sub_initiative_updates: s.include_sub_initiative_updates,
        active: s.active,
    }
}

#[derive(Default)]
pub struct NotificationMutation;

#[Object]
impl NotificationMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_notificationMutationReady")]
    async fn notification_mutation_ready(&self) -> bool {
        true
    }

    /// Gelesen/ungelesen, Snooze/Unsnooze.
    async fn notification_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: NotificationUpdateInput,
    ) -> async_graphql::Result<NotificationPayload> {
        let id = parse_id(&id).gql(ctx)?;
        let snoozed = match (&input.snoozed_until_at, input.unsnooze) {
            (Some(_), Some(true)) => {
                return Err(async_graphql::Error::new(
                    "Use either snoozedUntilAt or unsnooze, not both.",
                ));
            }
            (Some(s), _) => Some(Some(
                DateTime::parse_from_rfc3339(s)
                    .map_err(|_| async_graphql::Error::new("snoozedUntilAt must be ISO-8601."))?
                    .with_timezone(&Utc),
            )),
            (None, Some(true)) => Some(None),
            (None, _) => None,
        };
        let item = app(ctx)?
            .mark_notification(
                request_ctx(ctx)?,
                id,
                MarkNotificationInput {
                    read: input.read,
                    snoozed_until_at: snoozed,
                },
            )
            .await
            .gql(ctx)?;
        Ok(NotificationPayload {
            success: true,
            notification: Some(item.into()),
        })
    }

    /// Alle Notifications als gelesen markieren (`filter`: all, read, completed).
    async fn notification_mark_read_all(
        &self,
        ctx: &Context<'_>,
        filter: Option<String>,
    ) -> async_graphql::Result<NotificationBatchActionPayload> {
        let count = app(ctx)?
            .mark_all_notifications_read(request_ctx(ctx)?, filter.as_deref())
            .await
            .gql(ctx)?;
        Ok(NotificationBatchActionPayload {
            success: true,
            count: i64::try_from(count).unwrap_or(i64::MAX),
        })
    }

    /// Archiviert die Gruppe der Notification (`archive: false` = Undo).
    async fn notification_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
        archive: Option<bool>,
    ) -> async_graphql::Result<NotificationPayload> {
        let id = parse_id(&id).gql(ctx)?;
        let item = app(ctx)?
            .archive_notification(request_ctx(ctx)?, id, archive.unwrap_or(true))
            .await
            .gql(ctx)?;
        Ok(NotificationPayload {
            success: true,
            notification: Some(item.into()),
        })
    }

    /// Archiviert alle (`filter`: all, read, completed).
    async fn notification_archive_all(
        &self,
        ctx: &Context<'_>,
        filter: Option<String>,
    ) -> async_graphql::Result<NotificationBatchActionPayload> {
        let count = app(ctx)?
            .archive_all_notifications(request_ctx(ctx)?, filter.as_deref())
            .await
            .gql(ctx)?;
        Ok(NotificationBatchActionPayload {
            success: true,
            count: i64::try_from(count).unwrap_or(i64::MAX),
        })
    }

    /// Issue abonnieren (`user`: anderen User hinzufügen).
    async fn issue_subscribe(
        &self,
        ctx: &Context<'_>,
        id: String,
        user: Option<String>,
    ) -> async_graphql::Result<IssueSubscriptionPayload> {
        let r = app(ctx)?
            .subscribe_issue(request_ctx(ctx)?, &id, user.as_deref())
            .await
            .gql(ctx)?;
        Ok(IssueSubscriptionPayload {
            success: true,
            issue_id: r.issue_id.to_string(),
            user_id: r.user_id.to_string(),
            changed: r.changed,
        })
    }

    /// Issue-Abo beenden.
    async fn issue_unsubscribe(
        &self,
        ctx: &Context<'_>,
        id: String,
        user: Option<String>,
    ) -> async_graphql::Result<IssueSubscriptionPayload> {
        let r = app(ctx)?
            .unsubscribe_issue(request_ctx(ctx)?, &id, user.as_deref())
            .await
            .gql(ctx)?;
        Ok(IssueSubscriptionPayload {
            success: true,
            issue_id: r.issue_id.to_string(),
            user_id: r.user_id.to_string(),
            changed: r.changed,
        })
    }

    /// Objekt-Abo anlegen oder ändern (Upsert je Ziel und Kontext).
    async fn notification_subscription_update(
        &self,
        ctx: &Context<'_>,
        input: NotificationSubscriptionInput,
    ) -> async_graphql::Result<NotificationSubscriptionGql> {
        let kind = parse_kind(&input.target_type).gql(ctx)?;
        let target = Uuid::parse_str(input.target_id.trim())
            .map_err(|_| async_graphql::Error::new("targetId must be a UUID."))?;
        let sub = app(ctx)?
            .set_notification_subscription(
                request_ctx(ctx)?,
                SetObjectSubscriptionInput {
                    kind,
                    target,
                    context_view_type: input.context_view_type,
                    types: input.notification_subscription_types,
                    include_sub_initiative_updates: input.include_sub_initiative_updates,
                    active: input.active,
                },
            )
            .await
            .gql(ctx)?;
        Ok(sub_gql(sub))
    }

    /// Eigene Einstellungen ändern (die Inbox ist nicht abschaltbar).
    async fn notification_settings_update(
        &self,
        ctx: &Context<'_>,
        input: NotificationSettingsInput,
    ) -> async_graphql::Result<NotificationSettingsGql> {
        let s = app(ctx)?
            .update_notification_settings(
                request_ctx(ctx)?,
                NotificationSettingsPatch {
                    email_enabled: input.email_enabled,
                    email_format: input.email_format,
                    email_only_priority: input.email_only_priority,
                    email_hold_outside_work_hours: input.email_hold_outside_work_hours,
                    email_immediate_urgent: input.email_immediate_urgent,
                    due_date_reminders: input.due_date_reminders,
                    priority_categories: input.priority_categories,
                    channels: input
                        .channels
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| ChannelPreference {
                            category: c.category,
                            channel: c.channel,
                            enabled: c.enabled,
                        })
                        .collect(),
                },
            )
            .await
            .gql(ctx)?;
        Ok(s.into())
    }
}

#[cfg(test)]
mod tests {
    use crate::graphql::build_schema;
    use async_graphql::Request;
    use lineal_app::ports::Store;
    use lineal_db::PgStore;
    use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
    use std::sync::Arc;

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn inbox_roundtrip_and_isolation(pool: sqlx::PgPool) {
        let store = PgStore::new(pool.clone());
        let d = SeedData::acme();
        let mut tx = store.begin(d.org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let state = lineal_testkit::seed::identity::seed_workflow_state_ids(1)[1].0;
        let issue = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) VALUES ($1,$2,$3,77,'Login',$4,$5)")
            .bind(issue).bind(d.org.0).bind(d.eng.0).bind(state).bind(d.alice.0)
            .execute(&pool).await.unwrap();
        let nid = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id, issue_id, dedup_key) VALUES ($1,$2,$3,'issueAssignedToYou','assignments','issue',$4,$5,'k')")
            .bind(nid).bind(d.org.0).bind(d.bob.0).bind(d.alice.0).bind(issue)
            .execute(&pool).await.unwrap();
        let schema = build_schema(test_app(Arc::new(store)));
        let run = |who: SeedUser, q: String| {
            let schema = schema.clone();
            async move { schema.execute(Request::new(q).data(ctx_for(who))).await }
        };
        let r = run(SeedUser::Bob, "{ notifications { nodes { id type issueIdentifier actorName } pageInfo { hasNextPage } } notificationsUnreadCount }".into()).await;
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let j = r.data.into_json().unwrap();
        assert_eq!(j["notifications"]["nodes"][0]["type"], "issueAssignedToYou");
        assert_eq!(j["notifications"]["nodes"][0]["issueIdentifier"], "ENG-77");
        assert_eq!(j["notificationsUnreadCount"], 1);
        // Fremder User: leer und nicht markierbar.
        let r = run(SeedUser::Alice, "{ notifications { nodes { id } } }".into()).await;
        assert!(
            r.data.into_json().unwrap()["notifications"]["nodes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let r = run(SeedUser::Alice, format!("mutation {{ notificationUpdate(id: \"{nid}\", input: {{ read: true }}) {{ success }} }}")).await;
        assert_eq!(
            r.errors[0]
                .extensions
                .as_ref()
                .unwrap()
                .get("code")
                .unwrap()
                .to_string(),
            "\"not_found\""
        );
        let r = run(SeedUser::Bob, format!("mutation {{ notificationUpdate(id: \"{nid}\", input: {{ read: true }}) {{ success notification {{ readAt }} }} }}")).await;
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let r = run(SeedUser::Bob, "mutation { notificationArchiveAll { count } notificationSettingsUpdate(input: { emailFormat: \"immediate\" }) { emailFormat } }".into()).await;
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let j = r.data.into_json().unwrap();
        assert_eq!(j["notificationArchiveAll"]["count"], 1);
        assert_eq!(j["notificationSettingsUpdate"]["emailFormat"], "immediate");
    }
}
