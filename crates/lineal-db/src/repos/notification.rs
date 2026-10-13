//! `NotificationRepo` für `PgTx` (WP08).

use super::user::parse_role;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::NotificationRepo;
use lineal_app::ports::repos::notification::*;
use lineal_app::ports::repos::user::{Page, PageCursor, PageRequest};
use lineal_domain::ids::{CommentId, IssueId, NotificationId, ProjectId, TeamId, UserId};
use serde_json::Value;
use uuid::Uuid;

const VIEW_SELECT: &str = "SELECT n.id, n.user_id, n.type AS kind, n.category, n.target_type, n.actor_id, \
    n.issue_id, n.team_id, n.project_id, n.project_update_id, n.project_milestone_id, n.initiative_id, \
    n.initiative_update_id, n.document_id, n.comment_id, n.parent_comment_id, n.reaction_emoji, n.read_at, \
    n.snoozed_until_at, n.unsnoozed_at, n.emailed_at, n.is_priority, n.group_key, n.bundled_types, n.created_at, n.updated_at, \
    n.archived_at, a.name AS actor_name, i.identifier AS issue_identifier, i.team_id AS issue_team_id, i.title AS issue_title, i.priority AS issue_priority, i.due_date AS issue_due_date, ws.name AS issue_state_name, ws.type AS issue_state_type, \
    p.name AS project_name, ini.name AS initiative_name, d.title AS document_title, u.email AS recipient_email \
    FROM notification n \
    JOIN \"user\" u ON u.organization_id = n.organization_id AND u.id = n.user_id \
    LEFT JOIN \"user\" a ON a.organization_id = n.organization_id AND a.id = n.actor_id \
    LEFT JOIN issue i ON i.organization_id = n.organization_id AND i.id = n.issue_id \
    LEFT JOIN workflow_state ws ON ws.organization_id = i.organization_id AND ws.id = i.state_id \
    LEFT JOIN project p ON p.organization_id = n.organization_id AND p.id = n.project_id \
    LEFT JOIN initiative ini ON ini.organization_id = n.organization_id AND ini.id = n.initiative_id \
    LEFT JOIN document d ON d.organization_id = n.organization_id AND d.id = n.document_id ";

/// Der Kommentar, auf den sich die Notification bezieht, ist nicht gelöscht (B6.16).
const ALIVE_COMMENT_SQL: &str = "(n.comment_id IS NULL OR EXISTS (SELECT 1 FROM comment dc \
     WHERE dc.organization_id = n.organization_id AND dc.id = n.comment_id AND dc.archived_at IS NULL AND dc.deleted_at IS NULL))";

/// Bezugsobjekt sichtbar (`$vis` = Array sichtbarer Team-IDs).
fn visible_sql(vis: &str) -> String {
    format!(
        "(n.issue_id IS NULL OR i.team_id = ANY({vis}) OR EXISTS (SELECT 1 FROM issue_share ish WHERE ish.organization_id = n.organization_id AND ish.issue_id = n.issue_id AND ish.user_id = n.user_id)) \
         AND (i.id IS NULL OR NOT i.trashed) AND {ALIVE_COMMENT_SQL} \
         AND (n.target_type <> 'project' OR EXISTS (SELECT 1 FROM project_team pt \
              WHERE pt.project_id = n.project_id AND pt.team_id = ANY({vis}))) \
         AND {}",
        doc_visible_sql("n", vis)
    )
}

/// Dokument-Notifications: das Dokument muss über sein Elternobjekt (Issue, Projekt, Team) sichtbar
/// sein; Dokumente ohne Teambezug (Initiative, Workspace) sind für Nicht-Gäste immer sichtbar
/// (der Fan-out erzeugt sie nicht für Gäste).
fn doc_visible_sql(n: &str, vis: &str) -> String {
    format!(
        "({n}.document_id IS NULL OR EXISTS (SELECT 1 FROM document dv WHERE dv.id = {n}.document_id AND ( \
           (dv.issue_id IS NOT NULL AND EXISTS (SELECT 1 FROM issue dvi WHERE dvi.id = dv.issue_id AND dvi.team_id = ANY({vis}))) \
           OR (dv.issue_id IS NULL AND dv.project_id IS NOT NULL AND EXISTS (SELECT 1 FROM project_team dvp \
                WHERE dvp.project_id = dv.project_id AND dvp.team_id = ANY({vis}))) \
           OR (dv.issue_id IS NULL AND dv.project_id IS NULL AND dv.team_id IS NOT NULL AND dv.team_id = ANY({vis})) \
           OR (dv.issue_id IS NULL AND dv.project_id IS NULL AND dv.team_id IS NULL))))"
    )
}

/// Wie [`visible_sql`], aber ohne Joins (für UPDATE-Statements).
fn visible_sub_sql(vis: &str) -> String {
    format!(
        "((n.issue_id IS NULL OR EXISTS (SELECT 1 FROM issue vi WHERE vi.id = n.issue_id AND NOT vi.trashed \
              AND (vi.team_id = ANY({vis}) OR EXISTS (SELECT 1 FROM issue_share ish \
                   WHERE ish.organization_id = n.organization_id AND ish.issue_id = n.issue_id AND ish.user_id = n.user_id)))) \
         AND {ALIVE_COMMENT_SQL} \
         AND (n.target_type <> 'project' OR EXISTS (SELECT 1 FROM project_team pt \
              WHERE pt.project_id = n.project_id AND pt.team_id = ANY({vis}))) \
         AND {})",
        doc_visible_sql("n", vis)
    )
}

/// Nicht (mehr) gesnoozt.
fn awake_sql(now: &str) -> String {
    format!(
        "(n.snoozed_until_at IS NULL OR n.snoozed_until_at <= {now} OR n.unsnoozed_at IS NOT NULL)"
    )
}

fn bulk_sql(filter: BulkFilter) -> &'static str {
    match filter {
        BulkFilter::All => "TRUE",
        BulkFilter::Read => {
            "NOT EXISTS (SELECT 1 FROM notification x WHERE x.user_id = n.user_id \
             AND x.group_key = n.group_key AND x.archived_at IS NULL AND x.read_at IS NULL)"
        }
        BulkFilter::Completed => {
            "EXISTS (SELECT 1 FROM issue ci WHERE ci.id = n.issue_id \
             AND ci.state_type IN ('completed','canceled','duplicate'))"
        }
    }
}

#[derive(sqlx::FromRow)]
struct ViewRow {
    id: Uuid,
    user_id: Uuid,
    kind: String,
    category: String,
    target_type: String,
    actor_id: Option<Uuid>,
    issue_id: Option<Uuid>,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
    project_update_id: Option<Uuid>,
    project_milestone_id: Option<Uuid>,
    initiative_id: Option<Uuid>,
    initiative_update_id: Option<Uuid>,
    document_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    parent_comment_id: Option<Uuid>,
    reaction_emoji: Option<String>,
    read_at: Option<DateTime<Utc>>,
    snoozed_until_at: Option<DateTime<Utc>>,
    unsnoozed_at: Option<DateTime<Utc>>,
    emailed_at: Option<DateTime<Utc>>,
    is_priority: bool,
    group_key: String,
    bundled_types: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
    actor_name: Option<String>,
    issue_identifier: Option<String>,
    issue_team_id: Option<Uuid>,
    issue_title: Option<String>,
    issue_priority: Option<i32>,
    issue_due_date: Option<chrono::NaiveDate>,
    issue_state_name: Option<String>,
    issue_state_type: Option<String>,
    project_name: Option<String>,
    initiative_name: Option<String>,
    document_title: Option<String>,
    recipient_email: String,
}

impl From<ViewRow> for NotificationView {
    fn from(r: ViewRow) -> Self {
        Self {
            n: Notification {
                id: NotificationId(r.id),
                user_id: UserId(r.user_id),
                kind: r.kind,
                category: r.category,
                target_type: r.target_type,
                actor_id: r.actor_id.map(UserId),
                issue_id: r.issue_id.map(IssueId),
                team_id: r.team_id.map(TeamId),
                project_id: r.project_id.map(ProjectId),
                project_update_id: r.project_update_id,
                project_milestone_id: r.project_milestone_id,
                initiative_id: r.initiative_id,
                initiative_update_id: r.initiative_update_id,
                document_id: r.document_id,
                comment_id: r.comment_id.map(CommentId),
                parent_comment_id: r.parent_comment_id.map(CommentId),
                reaction_emoji: r.reaction_emoji,
                read_at: r.read_at,
                snoozed_until_at: r.snoozed_until_at,
                unsnoozed_at: r.unsnoozed_at,
                emailed_at: r.emailed_at,
                is_priority: r.is_priority,
                group_key: r.group_key,
                bundled_types: r.bundled_types,
                created_at: r.created_at,
                updated_at: r.updated_at,
                archived_at: r.archived_at,
            },
            actor_name: r.actor_name,
            issue_identifier: r.issue_identifier,
            issue_team_id: r.issue_team_id.map(TeamId),
            issue_title: r.issue_title,
            issue_priority: r.issue_priority,
            issue_due_date: r.issue_due_date,
            issue_state_name: r.issue_state_name,
            issue_state_type: r.issue_state_type,
            project_name: r.project_name,
            initiative_name: r.initiative_name,
            document_title: r.document_title,
            recipient_email: r.recipient_email,
        }
    }
}

fn uuids<T: Copy + Into<Uuid>>(ids: &[T]) -> Vec<Uuid> {
    ids.iter().map(|i| (*i).into()).collect()
}

#[derive(sqlx::FromRow)]
struct IssueFactsRow {
    id: Uuid,
    team_id: Uuid,
    identifier: String,
    title: String,
    creator_id: Option<Uuid>,
    assignee_id: Option<Uuid>,
    project_id: Option<Uuid>,
    cycle_id: Option<Uuid>,
    state_type: String,
    priority: i32,
    due_date: Option<NaiveDate>,
}

impl From<IssueFactsRow> for IssueFacts {
    fn from(r: IssueFactsRow) -> Self {
        Self {
            id: IssueId(r.id),
            team_id: TeamId(r.team_id),
            identifier: r.identifier,
            title: r.title,
            creator_id: r.creator_id.map(UserId),
            assignee_id: r.assignee_id.map(UserId),
            project_id: r.project_id.map(ProjectId),
            cycle_id: r.cycle_id,
            state_type: r.state_type,
            priority: r.priority,
            due_date: r.due_date,
        }
    }
}

const ISSUE_FACTS: &str = "SELECT id, team_id, identifier, title, creator_id, assignee_id, project_id, \
    cycle_id, state_type, priority, due_date FROM issue WHERE organization_id = $1 AND ";

type PrefsRow = (
    Uuid,
    String,
    Option<String>,
    String,
    bool,
    Vec<String>,
    Value,
);

fn prefs_from(r: PrefsRow) -> UserNotificationPrefs {
    UserNotificationPrefs {
        user_id: UserId(r.0),
        email: r.1,
        timezone: r.2,
        email_format: r.3,
        due_date_reminders: r.4,
        priority_categories: r.5,
        settings: r.6,
    }
}

const REMINDER_COLS: &str = "id, user_id, target_type, COALESCE(issue_id, project_id, initiative_id, document_id), \
     remind_at, fired_at";
const REMINDER_TARGET: &str = "COALESCE(issue_id, project_id, initiative_id, document_id)";

type ReminderRow = (
    Uuid,
    Uuid,
    String,
    Uuid,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn reminder_of(r: ReminderRow) -> Reminder {
    Reminder {
        id: r.0,
        user_id: UserId(r.1),
        target_type: r.2,
        target_id: r.3,
        remind_at: r.4,
        fired_at: r.5,
    }
}

fn reminder_cols(
    target_type: &str,
    id: Uuid,
) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>) {
    match target_type {
        "issue" => (Some(id), None, None, None),
        "project" => (None, Some(id), None, None),
        "initiative" => (None, None, Some(id), None),
        _ => (None, None, None, Some(id)),
    }
}

const PREFS_COLS: &str = "id, email, timezone, email_format, due_date_reminders, \
    notification_priority_categories, notification_settings";

type SubRow = (
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Vec<String>,
    bool,
    bool,
);

const SUB_COLS: &str = "id, subscriber_id, target_type, team_id, project_id, initiative_id, cycle_id, \
    COALESCE(label_id, user_id), COALESCE(context_view_type, user_context_view_type), \
    notification_subscription_types, include_sub_initiative_updates, active";

fn kind_of(s: &str) -> Option<ObjectKind> {
    Some(match s {
        "team" => ObjectKind::Team,
        "project" => ObjectKind::Project,
        "initiative" => ObjectKind::Initiative,
        "cycle" => ObjectKind::Cycle,
        "label" => ObjectKind::Label,
        "user" => ObjectKind::User,
        _ => return None,
    })
}

fn sub_from(r: SubRow) -> Option<ObjectSubscription> {
    let target = r.3.or(r.4).or(r.5).or(r.6).or(r.7)?;
    Some(ObjectSubscription {
        id: r.0,
        subscriber_id: UserId(r.1),
        kind: kind_of(&r.2)?,
        target_id: target,
        context_view_type: r.8,
        types: r.9,
        include_sub_initiative_updates: r.10,
        active: r.11,
    })
}

#[async_trait]
impl NotificationRepo for PgTx {
    async fn insert_notification(&mut self, n: &NewNotification) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id, \
             issue_id, team_id, project_id, project_update_id, project_milestone_id, initiative_id, \
             initiative_update_id, document_id, comment_id, parent_comment_id, reaction_emoji, is_priority, \
             dedup_key, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$21) \
             ON CONFLICT (user_id, dedup_key) DO NOTHING",
        )
        .bind(n.id.0)
        .bind(self.org.0)
        .bind(n.user_id.0)
        .bind(&n.kind)
        .bind(&n.category)
        .bind(&n.target_type)
        .bind(n.actor_id.map(|a| a.0))
        .bind(n.issue_id.map(|a| a.0))
        .bind(n.team_id.map(|a| a.0))
        .bind(n.project_id.map(|a| a.0))
        .bind(n.project_update_id)
        .bind(n.project_milestone_id)
        .bind(n.initiative_id)
        .bind(n.initiative_update_id)
        .bind(n.document_id)
        .bind(n.comment_id.map(|a| a.0))
        .bind(n.parent_comment_id.map(|a| a.0))
        .bind(&n.reaction_emoji)
        .bind(n.is_priority)
        .bind(&n.dedup_key)
        .bind(n.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Ok(false);
        }
        // Neue Aktivität in der Gruppe beendet ein laufendes Snooze (docs/05 §6.4 Regel 3).
        sqlx::query(
            "UPDATE notification SET unsnoozed_at = $3, updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND id <> $4 \
               AND group_key = (SELECT g.group_key FROM notification g WHERE g.id = $4 AND g.organization_id = $1 AND g.user_id = $2) \
               AND snoozed_until_at > $3 AND unsnoozed_at IS NULL AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(n.user_id.0)
        .bind(n.now)
        .bind(n.id.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(true)
    }

    async fn dedup_seen(&mut self, user: UserId, dedup_key: &str) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM notification WHERE organization_id = $1 AND user_id = $2 \
                            AND dedup_key = $3) \
                 OR EXISTS (SELECT 1 FROM notification_merge m JOIN notification n ON n.id = m.notification_id \
                            WHERE n.organization_id = $1 AND m.user_id = $2 AND m.dedup_key = $3)",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(dedup_key)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_bundle_target(
        &mut self,
        user: UserId,
        group_key: &str,
        actor: UserId,
        since: DateTime<Utc>,
    ) -> AppResult<Option<BundleTarget>> {
        let row: Option<(Uuid, String, Vec<String>, bool)> = sqlx::query_as(
            "SELECT id, type, bundled_types, is_priority FROM notification \
             WHERE organization_id = $1 AND user_id = $2 AND group_key = $3 AND actor_id = $4 \
               AND archived_at IS NULL AND created_at >= $5 \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(group_key)
        .bind(actor.0)
        .bind(since)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(
            row.map(|(id, kind, bundled_types, is_priority)| BundleTarget {
                id: NotificationId(id),
                kind,
                bundled_types,
                is_priority,
            }),
        )
    }

    async fn merge_notification(&mut self, n: &NewNotification, m: &BundleMerge) -> AppResult<()> {
        // Nur in eine Notification desselben Empfängers und Workspace zusammenführen: die Zeile
        // wird nur vermerkt, wenn das Ziel (org, user) gehört; sonst (oder bei Wiederholung) No-op.
        let claimed = sqlx::query(
            "INSERT INTO notification_merge (user_id, dedup_key, notification_id) \
             SELECT t.user_id, $2, t.id FROM notification t \
             WHERE t.id = $3 AND t.organization_id = $4 AND t.user_id = $1 \
             ON CONFLICT DO NOTHING",
        )
        .bind(n.user_id.0)
        .bind(&n.dedup_key)
        .bind(m.target.0)
        .bind(self.org.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if claimed.rows_affected() == 0 {
            return Ok(());
        }
        let (comment, parent, emoji) = if m.take_refs {
            (
                n.comment_id.map(|c| c.0),
                n.parent_comment_id.map(|c| c.0),
                n.reaction_emoji.clone(),
            )
        } else {
            (None, None, None)
        };
        sqlx::query(
            "UPDATE notification SET type = $3, category = $4, bundled_types = $5, \
               is_priority = $6, read_at = NULL, updated_at = $7, version = version + 1, \
               comment_id = CASE WHEN $8 THEN $9 ELSE comment_id END, \
               parent_comment_id = CASE WHEN $8 THEN $10 ELSE parent_comment_id END, \
               reaction_emoji = CASE WHEN $8 THEN $11 ELSE reaction_emoji END, \
               unsnoozed_at = CASE WHEN snoozed_until_at > $7 AND unsnoozed_at IS NULL THEN $7 \
                                   ELSE unsnoozed_at END \
             WHERE organization_id = $1 AND id = $2 AND user_id = $12",
        )
        .bind(self.org.0)
        .bind(m.target.0)
        .bind(&m.kind)
        .bind(&m.category)
        .bind(&m.bundled_types)
        .bind(m.is_priority)
        .bind(n.now)
        .bind(m.take_refs)
        .bind(comment)
        .bind(parent)
        .bind(emoji)
        .bind(n.user_id.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        // Neue Aktivität beendet ein Snooze der übrigen Notifications der Gruppe (docs/05 §6.4 Regel 3).
        sqlx::query(
            "UPDATE notification SET unsnoozed_at = $3, updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND id <> $4 \
               AND group_key = (SELECT g.group_key FROM notification g WHERE g.id = $4 AND g.organization_id = $1 AND g.user_id = $2) \
               AND snoozed_until_at > $3 AND unsnoozed_at IS NULL AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(n.user_id.0)
        .bind(n.now)
        .bind(m.target.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_notification(
        &mut self,
        user: UserId,
        id: NotificationId,
    ) -> AppResult<Option<NotificationView>> {
        let sql =
            format!("{VIEW_SELECT} WHERE n.organization_id = $1 AND n.user_id = $2 AND n.id = $3");
        let row: Option<ViewRow> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(user.0)
            .bind(id.0)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn list_notifications(
        &mut self,
        user: UserId,
        query: &InboxQuery,
        now: DateTime<Utc>,
        page: &PageRequest,
    ) -> AppResult<Page<NotificationView>> {
        let mut sql = format!(
            "{VIEW_SELECT} WHERE n.organization_id = $1 AND n.user_id = $2 AND {} ",
            visible_sql("$3")
        );
        if !query.include_archived {
            sql.push_str("AND n.archived_at IS NULL ");
        }
        if !query.include_snoozed {
            sql.push_str(&format!("AND {} ", awake_sql("$4")));
        }
        if query.unread_only {
            sql.push_str("AND n.read_at IS NULL ");
        }
        sql.push_str("AND ($5::timestamptz IS NULL OR (n.created_at, n.id) < ($5, $6)) ");
        sql.push_str("ORDER BY n.created_at DESC, n.id DESC LIMIT $7");
        let rows: Vec<ViewRow> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(user.0)
            .bind(uuids(&query.visible_teams))
            .bind(now)
            .bind(page.cursor.map(|c| c.key))
            .bind(page.cursor.map_or(Uuid::nil(), |c| c.id))
            .bind(page.limit + 1)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let mut items: Vec<NotificationView> = rows.into_iter().map(Into::into).collect();
        let has_next = items.len() as i64 > page.limit;
        if has_next {
            items.truncate(page.limit as usize);
        }
        let next = if has_next {
            items.last().map(|v| PageCursor {
                key: v.n.created_at,
                id: v.n.id.0,
            })
        } else {
            None
        };
        Ok(Page { items, next })
    }

    async fn count_unread_groups(
        &mut self,
        user: UserId,
        visible_teams: &[TeamId],
        now: DateTime<Utc>,
    ) -> AppResult<i64> {
        let sql = format!(
            "SELECT count(*) FROM (SELECT n.group_key FROM notification n \
             LEFT JOIN issue i ON i.organization_id = n.organization_id AND i.id = n.issue_id \
             WHERE n.organization_id = $1 AND n.user_id = $2 AND n.archived_at IS NULL AND {} AND {} \
             GROUP BY n.group_key HAVING bool_or(n.read_at IS NULL)) g",
            visible_sql("$3"),
            awake_sql("$4")
        );
        sqlx::query_scalar(&sql)
            .bind(self.org.0)
            .bind(user.0)
            .bind(uuids(visible_teams))
            .bind(now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)
    }

    async fn set_read(
        &mut self,
        user: UserId,
        id: NotificationId,
        read: bool,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<NotificationId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE notification SET read_at = CASE WHEN $4 THEN $5 ELSE NULL END, updated_at = $5, \
             version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND archived_at IS NULL AND (read_at IS NULL) = $4 \
               AND group_key = (SELECT g.group_key FROM notification g \
                                WHERE g.organization_id = $1 AND g.user_id = $2 AND g.id = $3) \
             RETURNING id",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(id.0)
        .bind(read)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(ids.into_iter().map(NotificationId).collect())
    }

    async fn mark_group_read(
        &mut self,
        user: UserId,
        group_key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE notification SET read_at = $4, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND group_key = $3 \
               AND read_at IS NULL AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(group_key)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn mark_all_read(
        &mut self,
        user: UserId,
        visible_teams: &[TeamId],
        filter: BulkFilter,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<NotificationId>> {
        let sql = format!(
            "UPDATE notification n SET read_at = $3, updated_at = $3, version = version + 1 \
             WHERE n.organization_id = $1 AND n.user_id = $2 AND n.read_at IS NULL \
               AND n.archived_at IS NULL AND {} AND {} RETURNING n.id",
            visible_sub_sql("$4"),
            bulk_sql(filter)
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(&sql)
            .bind(self.org.0)
            .bind(user.0)
            .bind(now)
            .bind(uuids(visible_teams))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(ids.into_iter().map(NotificationId).collect())
    }

    async fn snooze_group(
        &mut self,
        user: UserId,
        group_key: &str,
        until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE notification SET snoozed_until_at = $4, unsnoozed_at = NULL, updated_at = $5, \
             version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND group_key = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(group_key)
        .bind(until)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn set_group_archived(
        &mut self,
        user: UserId,
        group_key: &str,
        archive: bool,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE notification SET archived_at = CASE WHEN $4 THEN $5 ELSE NULL END, updated_at = $5, \
             version = version + 1 \
             WHERE organization_id = $1 AND user_id = $2 AND group_key = $3 \
               AND (archived_at IS NULL) = $4",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(group_key)
        .bind(archive)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn archive_all(
        &mut self,
        user: UserId,
        visible_teams: &[TeamId],
        filter: BulkFilter,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<NotificationId>> {
        let sql = format!(
            "UPDATE notification n SET archived_at = $3, updated_at = $3, version = version + 1 \
             WHERE n.organization_id = $1 AND n.user_id = $2 AND n.archived_at IS NULL AND {} AND {} RETURNING n.id",
            visible_sub_sql("$4"),
            bulk_sql(filter)
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(&sql)
            .bind(self.org.0)
            .bind(user.0)
            .bind(now)
            .bind(uuids(visible_teams))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(ids.into_iter().map(NotificationId).collect())
    }

    async fn unsnooze_due(&mut self, now: DateTime<Utc>) -> AppResult<u64> {
        // Bewusst mandantenübergreifend; nur über `Store::begin_system` aufzurufen.
        let res = sqlx::query(
            "UPDATE notification SET unsnoozed_at = $1, read_at = NULL, updated_at = $1, version = version + 1 \
             WHERE snoozed_until_at IS NOT NULL AND snoozed_until_at <= $1 AND unsnoozed_at IS NULL \
               AND archived_at IS NULL",
        )
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn archive_over_cap(&mut self, now: DateTime<Utc>) -> AppResult<u64> {
        // Bewusst mandantenübergreifend; nur über `Store::begin_system` aufzurufen.
        let res = sqlx::query(
            "WITH over_cap AS ( \
                 SELECT n.organization_id, n.user_id, \
                        COALESCE((o.limits->>'maxUnarchivedNotificationsPerUser')::int, 2000) AS cap \
                 FROM notification n JOIN organization o ON o.id = n.organization_id \
                 WHERE n.archived_at IS NULL \
                 GROUP BY n.organization_id, n.user_id, o.limits \
                 HAVING count(*) > COALESCE((o.limits->>'maxUnarchivedNotificationsPerUser')::int, 2000)), \
             ranked AS ( \
                 SELECT n.id, oc.cap, row_number() OVER (PARTITION BY n.user_id \
                        ORDER BY COALESCE(n.snoozed_until_at > $1 AND n.unsnoozed_at IS NULL, false) DESC, \
                                 n.created_at DESC, n.id DESC) AS rn \
                 FROM notification n JOIN over_cap oc \
                   ON oc.organization_id = n.organization_id AND oc.user_id = n.user_id \
                 WHERE n.archived_at IS NULL) \
             UPDATE notification SET archived_at = $1, updated_at = $1, version = version + 1 \
             WHERE id IN (SELECT id FROM ranked WHERE rn > cap)",
        )
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn purge_archived(&mut self, before: DateTime<Utc>) -> AppResult<u64> {
        let res = sqlx::query(
            "DELETE FROM notification WHERE archived_at IS NOT NULL AND archived_at < $1",
        )
        .bind(before)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn list_email_candidates(
        &mut self,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<NotificationView>> {
        let sql = format!(
            "{VIEW_SELECT} WHERE n.organization_id = $1 AND n.read_at IS NULL AND n.emailed_at IS NULL \
             AND n.archived_at IS NULL AND u.active AND n.created_at >= $2 AND {} \
             AND (i.id IS NULL OR NOT i.trashed) AND {ALIVE_COMMENT_SQL} \
             ORDER BY n.created_at, n.id LIMIT $4",
            awake_sql("$3")
        );
        let rows: Vec<ViewRow> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(since)
            .bind(now)
            .bind(limit)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn mark_emailed(&mut self, ids: &[NotificationId], now: DateTime<Utc>) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE notification SET emailed_at = $3, updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(uuids(ids))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list_due_candidates(&mut self, due_until: NaiveDate) -> AppResult<Vec<DueCandidate>> {
        let rows: Vec<(Uuid, Uuid, Uuid, NaiveDate, Option<String>)> = sqlx::query_as(
            "SELECT i.id, i.team_id, i.assignee_id, i.due_date, u.timezone \
             FROM issue i JOIN \"user\" u ON u.organization_id = i.organization_id AND u.id = i.assignee_id \
             WHERE i.organization_id = $1 AND i.due_date IS NOT NULL AND i.due_date <= $2 \
               AND i.state_type NOT IN ('completed','canceled','duplicate') \
               AND i.archived_at IS NULL AND NOT i.trashed AND u.due_date_reminders AND u.active",
        )
        .bind(self.org.0)
        .bind(due_until)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(i, t, a, d, tz)| DueCandidate {
                issue_id: IssueId(i),
                team_id: TeamId(t),
                assignee_id: UserId(a),
                due_date: d,
                timezone: tz,
            })
            .collect())
    }

    async fn list_org_ids(&mut self) -> AppResult<Vec<Uuid>> {
        sqlx::query_scalar("SELECT id FROM organization ORDER BY id")
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)
    }

    async fn issue_facts(&mut self, id: IssueId) -> AppResult<Option<IssueFacts>> {
        let row: Option<IssueFactsRow> = sqlx::query_as(&format!("{ISSUE_FACTS}id = $2"))
            .bind(self.org.0)
            .bind(id.0)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn issue_facts_by_ref(&mut self, reference: &str) -> AppResult<Option<IssueFacts>> {
        if let Ok(id) = Uuid::parse_str(reference.trim()) {
            return self.issue_facts(IssueId(id)).await;
        }
        let row: Option<IssueFactsRow> = sqlx::query_as(&format!(
            "{ISSUE_FACTS}(identifier = upper($2) OR id = (SELECT issue_id FROM issue_identifier_alias \
             WHERE organization_id = $1 AND identifier = upper($2) AND issue_id IS NOT NULL)) \
             ORDER BY trashed, created_at DESC LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(reference.trim())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn comment_facts(&mut self, id: CommentId) -> AppResult<Option<CommentFacts>> {
        type Row = (
            Uuid,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, user_id, issue_id, project_id, parent_id, project_milestone_id, project_update_id, \
             initiative_id, initiative_update_id, document_id FROM comment \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(
            row.map(|(id, u, i, p, par, pm, pu, ini, iu, doc)| CommentFacts {
                id: CommentId(id),
                author_id: u.map(UserId),
                issue_id: i.map(IssueId),
                project_id: p.map(ProjectId),
                project_milestone_id: pm,
                project_update_id: pu,
                initiative_id: ini,
                initiative_update_id: iu,
                document_id: doc,
                parent_id: par.map(CommentId),
            }),
        )
    }

    async fn milestone_facts(&mut self, id: Uuid) -> AppResult<Option<MilestoneFacts>> {
        let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
            "SELECT project_id, description FROM project_milestone WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(p, d)| MilestoneFacts {
            id,
            project_id: ProjectId(p),
            description: d.unwrap_or_default(),
        }))
    }

    async fn initiative_update_facts(
        &mut self,
        id: Uuid,
    ) -> AppResult<Option<InitiativeUpdateFacts>> {
        let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT initiative_id, user_id, body FROM initiative_update WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(i, u, body)| InitiativeUpdateFacts {
            id,
            initiative_id: i,
            author_id: UserId(u),
            body,
        }))
    }

    async fn document_facts(&mut self, id: Uuid) -> AppResult<Option<DocumentFacts>> {
        type Row = (Option<Uuid>, bool, Option<Uuid>, Option<Uuid>, Option<Uuid>);
        let row: Option<Row> = sqlx::query_as(
            "SELECT d.creator_id, d.trashed, \
                    (SELECT i.team_id FROM issue i WHERE i.id = d.issue_id), d.project_id, d.team_id \
             FROM document d WHERE d.organization_id = $1 AND d.id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let Some((creator, trashed, issue_team, project, team)) = row else {
            return Ok(None);
        };
        let teams: Vec<Uuid> = if let Some(t) = issue_team {
            vec![t]
        } else if let Some(p) = project {
            sqlx::query_scalar(
                "SELECT team_id FROM project_team WHERE organization_id = $1 AND project_id = $2",
            )
            .bind(self.org.0)
            .bind(p)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?
        } else {
            team.into_iter().collect()
        };
        let workspace_only = teams.is_empty();
        Ok(Some(DocumentFacts {
            id,
            creator_id: creator.map(UserId),
            trashed,
            teams: teams.into_iter().map(TeamId).collect(),
            workspace_only,
        }))
    }

    async fn blocked_issues(&mut self, issue: IssueId) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT related_issue_id FROM issue_relation \
             WHERE organization_id = $1 AND issue_id = $2 AND type = 'blocks'",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn open_blockers(&mut self, issue: IssueId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM issue_relation r JOIN issue b ON b.id = r.issue_id \
             WHERE r.organization_id = $1 AND r.related_issue_id = $2 AND r.type = 'blocks' \
               AND b.state_type NOT IN ('completed','canceled','duplicate') AND NOT b.trashed",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn reaction_facts(&mut self, id: Uuid) -> AppResult<Option<ReactionFacts>> {
        type Row = (
            Uuid,
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT user_id, emoji, issue_id, comment_id, project_update_id, initiative_update_id \
             FROM reaction WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(u, e, i, c, pu, iu)| ReactionFacts {
            user_id: UserId(u),
            emoji: e,
            issue_id: i.map(IssueId),
            comment_id: c.map(CommentId),
            project_update_id: pu,
            initiative_update_id: iu,
        }))
    }

    async fn relation_facts(&mut self, id: Uuid) -> AppResult<Option<RelationFacts>> {
        let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT issue_id, related_issue_id, type FROM issue_relation WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(a, b, k)| RelationFacts {
            issue_id: IssueId(a),
            related_issue_id: IssueId(b),
            kind: k,
        }))
    }

    async fn project_update_facts(&mut self, id: Uuid) -> AppResult<Option<ProjectUpdateFacts>> {
        let row: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT id, project_id, user_id, body FROM project_update WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(id, p, u, body)| ProjectUpdateFacts {
            id,
            project_id: ProjectId(p),
            author_id: UserId(u),
            body,
        }))
    }

    async fn has_issue_notification(
        &mut self,
        user: UserId,
        issue: IssueId,
        kind: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM notification WHERE organization_id = $1 AND user_id = $2 \
             AND issue_id = $3 AND type = $4 AND archived_at IS NULL)",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(issue.0)
        .bind(kind)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn project_facts(&mut self, id: ProjectId) -> AppResult<Option<ProjectFacts>> {
        let row: Option<(Option<Uuid>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT lead_id, description, content FROM project \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(lead, d, c)| ProjectFacts {
            id,
            lead_id: lead.map(UserId),
            text: format!("{}\n{}", d.unwrap_or_default(), c.unwrap_or_default()),
        }))
    }

    async fn initiative_facts(&mut self, id: Uuid) -> AppResult<Option<InitiativeFacts>> {
        let row: Option<(Option<Uuid>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT owner_id, description, content FROM initiative \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(owner, d, c)| InitiativeFacts {
            id,
            owner_id: owner.map(UserId),
            text: format!("{}\n{}", d.unwrap_or_default(), c.unwrap_or_default()),
        }))
    }

    async fn project_team_ids(&mut self, project: ProjectId) -> AppResult<Vec<TeamId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT team_id FROM project_team WHERE organization_id = $1 AND project_id = $2",
        )
        .bind(self.org.0)
        .bind(project.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(TeamId).collect())
    }

    async fn fanout_users(&mut self, ids: &[UserId]) -> AppResult<Vec<FanoutUser>> {
        let rows: Vec<(Uuid, String, bool)> = sqlx::query_as(
            "SELECT id, role, active FROM \"user\" WHERE organization_id = $1 AND id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(uuids(ids))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|(id, role, active)| {
                Ok(FanoutUser {
                    id: UserId(id),
                    role: parse_role(&role)?,
                    active,
                })
            })
            .collect()
    }

    async fn subscribe_document(
        &mut self,
        document: Uuid,
        user: UserId,
        reason: SubscriptionReason,
        now: DateTime<Utc>,
    ) -> AppResult<SubscribeOutcome> {
        let existing: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT id, active FROM document_subscription WHERE organization_id = $1 \
             AND document_id = $2 AND user_id = $3",
        )
        .bind(self.org.0)
        .bind(document)
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        match existing {
            None => {
                let inserted = sqlx::query(
                    "INSERT INTO document_subscription (organization_id, document_id, user_id, active, \
                     reason, created_at, updated_at) VALUES ($1,$2,$3,true,$4,$5,$5) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(self.org.0)
                .bind(document)
                .bind(user.0)
                .bind(reason.as_str())
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                // Parallele Aufrufe: nur der Gewinner meldet `Created` (idempotent).
                if inserted.rows_affected() == 0 {
                    return Ok(SubscribeOutcome::Unchanged);
                }
                Ok(SubscribeOutcome::Created)
            }
            Some((id, false)) if reason.reactivates() => {
                sqlx::query(
                    "UPDATE document_subscription SET active = true, reason = $2, updated_at = $3, \
                     version = version + 1 WHERE id = $1 AND organization_id = $4 AND user_id = $5",
                )
                .bind(id)
                .bind(reason.as_str())
                .bind(now)
                .bind(self.org.0)
                .bind(user.0)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(SubscribeOutcome::Reactivated)
            }
            Some(_) => Ok(SubscribeOutcome::Unchanged),
        }
    }

    async fn unsubscribe_document(
        &mut self,
        document: Uuid,
        user: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE document_subscription SET active = false, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND document_id = $2 AND user_id = $3 AND active",
        )
        .bind(self.org.0)
        .bind(document)
        .bind(user.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn document_subscribers(&mut self, document: Uuid) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM document_subscription WHERE organization_id = $1 \
             AND document_id = $2 AND active AND archived_at IS NULL ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(document)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn is_document_subscriber(&mut self, document: Uuid, user: UserId) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM document_subscription WHERE organization_id = $1 \
             AND document_id = $2 AND user_id = $3 AND active AND archived_at IS NULL)",
        )
        .bind(self.org.0)
        .bind(document)
        .bind(user.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn subscribe_issue(
        &mut self,
        issue: IssueId,
        user: UserId,
        thread: Option<CommentId>,
        reason: SubscriptionReason,
        now: DateTime<Utc>,
    ) -> AppResult<SubscribeOutcome> {
        let existing: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT id, active FROM issue_subscription WHERE organization_id = $1 AND issue_id = $2 \
             AND user_id = $3 AND thread_comment_id IS NOT DISTINCT FROM $4",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .bind(thread.map(|t| t.0))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        match existing {
            None => {
                let inserted = sqlx::query(
                    "INSERT INTO issue_subscription (organization_id, issue_id, user_id, thread_comment_id, \
                     active, reason, created_at, updated_at) VALUES ($1,$2,$3,$4,true,$5,$6,$6) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(self.org.0)
                .bind(issue.0)
                .bind(user.0)
                .bind(thread.map(|t| t.0))
                .bind(reason.as_str())
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                // Parallele Aufrufe: nur der Gewinner meldet `Created` (idempotent).
                if inserted.rows_affected() == 0 {
                    return Ok(SubscribeOutcome::Unchanged);
                }
                Ok(SubscribeOutcome::Created)
            }
            Some((id, false)) if reason.reactivates() => {
                sqlx::query(
                    "UPDATE issue_subscription SET active = true, reason = $2, updated_at = $3, \
                     version = version + 1 WHERE id = $1",
                )
                .bind(id)
                .bind(reason.as_str())
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(SubscribeOutcome::Reactivated)
            }
            Some(_) => Ok(SubscribeOutcome::Unchanged),
        }
    }

    async fn unsubscribe_issue(
        &mut self,
        issue: IssueId,
        user: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE issue_subscription SET active = false, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND issue_id = $2 AND user_id = $3 AND active",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let was_active = res.rows_affected() > 0;
        // Ohne Datensatz einen inaktiven anlegen, damit Kommentieren nicht erneut abonniert.
        sqlx::query(
            "INSERT INTO issue_subscription (organization_id, issue_id, user_id, thread_comment_id, \
             active, reason, created_at, updated_at) \
             SELECT $1,$2,$3,NULL,false,'manual',$4,$4 \
             WHERE NOT EXISTS (SELECT 1 FROM issue_subscription WHERE issue_id = $2 AND user_id = $3 \
                               AND thread_comment_id IS NULL)",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(was_active)
    }

    async fn list_issue_subscriptions(
        &mut self,
        issue: IssueId,
    ) -> AppResult<Vec<IssueSubscription>> {
        let rows: Vec<(Uuid, Option<Uuid>, bool, String)> = sqlx::query_as(
            "SELECT user_id, thread_comment_id, active, reason FROM issue_subscription \
             WHERE organization_id = $1 AND issue_id = $2 ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(u, t, a, r)| IssueSubscription {
                user_id: UserId(u),
                thread_comment_id: t.map(CommentId),
                active: a,
                reason: r,
            })
            .collect())
    }

    async fn issue_subscribers(&mut self, issue: IssueId) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM issue_subscription WHERE organization_id = $1 AND issue_id = $2 \
             AND active AND thread_comment_id IS NULL",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn thread_subscribers(
        &mut self,
        issue: IssueId,
        thread: CommentId,
    ) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM issue_subscription WHERE organization_id = $1 AND issue_id = $2 \
             AND active AND thread_comment_id = $3",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(thread.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn thread_participants(&mut self, thread: CommentId) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM comment WHERE organization_id = $1 AND (id = $2 OR parent_id = $2) \
             AND user_id IS NOT NULL AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(thread.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn upsert_object_subscription(
        &mut self,
        sub: &UpsertObjectSubscription,
    ) -> AppResult<ObjectSubscription> {
        let col = sub.kind.column();
        let ctx_col = sub.kind.context_column();
        let existing: Option<Uuid> = sqlx::query_scalar(&format!(
            "SELECT id FROM notification_subscription WHERE organization_id = $1 AND subscriber_id = $2 \
             AND target_type = $3 AND {col} = $4 AND {ctx_col} IS NOT DISTINCT FROM $5"
        ))
        .bind(self.org.0)
        .bind(sub.subscriber_id.0)
        .bind(sub.kind.as_str())
        .bind(sub.target_id)
        .bind(&sub.context_view_type)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let id = match existing {
            Some(id) => {
                sqlx::query(
                    "UPDATE notification_subscription SET notification_subscription_types = $2, \
                     include_sub_initiative_updates = $3, active = $4, updated_at = $5, version = version + 1 \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(&sub.types)
                .bind(sub.include_sub_initiative_updates)
                .bind(sub.active)
                .bind(sub.now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                id
            }
            None => {
                sqlx::query(&format!(
                    "INSERT INTO notification_subscription (id, organization_id, subscriber_id, target_type, \
                     {col}, {ctx_col}, notification_subscription_types, \
                     include_sub_initiative_updates, active, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10)"
                ))
                .bind(sub.id)
                .bind(self.org.0)
                .bind(sub.subscriber_id.0)
                .bind(sub.kind.as_str())
                .bind(sub.target_id)
                .bind(&sub.context_view_type)
                .bind(&sub.types)
                .bind(sub.include_sub_initiative_updates)
                .bind(sub.active)
                .bind(sub.now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                sub.id
            }
        };
        let row: SubRow = sqlx::query_as(&format!(
            "SELECT {SUB_COLS} FROM notification_subscription WHERE id = $1"
        ))
        .bind(id)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sub_from(row)
            .ok_or_else(|| lineal_app::AppError::internal("invalid notification subscription"))
    }

    async fn list_object_subscriptions(
        &mut self,
        user: UserId,
    ) -> AppResult<Vec<ObjectSubscription>> {
        let rows: Vec<SubRow> = sqlx::query_as(&format!(
            "SELECT {SUB_COLS} FROM notification_subscription WHERE organization_id = $1 AND subscriber_id = $2 \
             AND archived_at IS NULL AND target_type IN ('team','project','initiative','cycle','label','user') \
             ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().filter_map(sub_from).collect())
    }

    async fn object_subscribers(
        &mut self,
        kind: ObjectKind,
        target: Uuid,
        ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        let col = kind.column();
        let rows: Vec<Uuid> = sqlx::query_scalar(&format!(
            "SELECT DISTINCT subscriber_id FROM notification_subscription WHERE organization_id = $1 \
             AND target_type = $2 AND {col} = $3 AND active AND archived_at IS NULL \
             AND $4 = ANY(notification_subscription_types)"
        ))
        .bind(self.org.0)
        .bind(kind.as_str())
        .bind(target)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn initiative_subscribers_of_project(
        &mut self,
        project: ProjectId,
        ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT ns.subscriber_id FROM notification_subscription ns \
             WHERE ns.organization_id = $1 AND ns.target_type = 'initiative' AND ns.active \
               AND ns.archived_at IS NULL AND $3 = ANY(ns.notification_subscription_types) \
               AND ns.initiative_id IN (SELECT initiative_id FROM initiative_to_project WHERE project_id = $2)",
        )
        .bind(self.org.0)
        .bind(project.0)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn label_subscribers_of_issue(
        &mut self,
        issue: IssueId,
        ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT ns.subscriber_id FROM notification_subscription ns \
             WHERE ns.organization_id = $1 AND ns.target_type = 'label' AND ns.active \
               AND ns.archived_at IS NULL AND $3 = ANY(ns.notification_subscription_types) \
               AND ns.label_id IN (SELECT label_id FROM issue_to_label WHERE issue_id = $2)",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn label_subscribers_of_labels(
        &mut self,
        labels: &[Uuid],
        ntype: &str,
    ) -> AppResult<Vec<UserId>> {
        if labels.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT ns.subscriber_id FROM notification_subscription ns \
             WHERE ns.organization_id = $1 AND ns.target_type = 'label' AND ns.active \
               AND ns.archived_at IS NULL AND $3 = ANY(ns.notification_subscription_types) \
               AND ns.label_id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(labels)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn label_team(&mut self, label: Uuid) -> AppResult<Option<Option<TeamId>>> {
        let row: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT team_id FROM issue_label WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(label)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|t| t.map(TeamId)))
    }

    async fn cycle_team(&mut self, cycle: Uuid) -> AppResult<Option<TeamId>> {
        let row: Option<Uuid> =
            sqlx::query_scalar("SELECT team_id FROM cycle WHERE organization_id = $1 AND id = $2")
                .bind(self.org.0)
                .bind(cycle)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
        Ok(row.map(TeamId))
    }

    async fn set_reminder(
        &mut self,
        id: Uuid,
        user: UserId,
        target_type: &str,
        target_id: Uuid,
        remind_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Reminder> {
        let (issue, project, initiative, document) = reminder_cols(target_type, target_id);
        let row: ReminderRow = sqlx::query_as(&format!(
            "INSERT INTO reminder (id, organization_id, user_id, target_type, issue_id, project_id, \
               initiative_id, document_id, remind_at, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10) \
             ON CONFLICT (organization_id, user_id, target_type, issue_id, project_id, initiative_id, document_id) \
             DO UPDATE SET remind_at = EXCLUDED.remind_at, fired_at = NULL, updated_at = $10, \
               version = reminder.version + 1 \
             RETURNING {REMINDER_COLS}"
        ))
        .bind(id)
        .bind(self.org.0)
        .bind(user.0)
        .bind(target_type)
        .bind(issue)
        .bind(project)
        .bind(initiative)
        .bind(document)
        .bind(remind_at)
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(reminder_of(row))
    }

    async fn clear_reminder(
        &mut self,
        user: UserId,
        target_type: &str,
        target_id: Uuid,
    ) -> AppResult<bool> {
        let r = sqlx::query(&format!(
            "DELETE FROM reminder WHERE organization_id = $1 AND user_id = $2 AND target_type = $3 \
             AND {REMINDER_TARGET} = $4"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .bind(target_type)
        .bind(target_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected() > 0)
    }

    async fn get_reminder(
        &mut self,
        user: UserId,
        target_type: &str,
        target_id: Uuid,
    ) -> AppResult<Option<Reminder>> {
        let row: Option<ReminderRow> = sqlx::query_as(&format!(
            "SELECT {REMINDER_COLS} FROM reminder WHERE organization_id = $1 AND user_id = $2 \
             AND target_type = $3 AND {REMINDER_TARGET} = $4 AND fired_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .bind(target_type)
        .bind(target_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(reminder_of))
    }

    async fn list_due_reminders(
        &mut self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<Reminder>> {
        let rows: Vec<ReminderRow> = sqlx::query_as(&format!(
            "SELECT {REMINDER_COLS} FROM reminder WHERE organization_id = $1 AND fired_at IS NULL \
             AND remind_at <= $2 ORDER BY remind_at, id LIMIT $3"
        ))
        .bind(self.org.0)
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(reminder_of).collect())
    }

    async fn mark_reminder_fired(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE reminder SET fired_at = $3, updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn reminder_target_alive(&mut self, target_type: &str, id: Uuid) -> AppResult<bool> {
        let table = match target_type {
            "issue" => "issue",
            "project" => "project",
            "initiative" => "initiative",
            _ => return Ok(true),
        };
        sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table} WHERE organization_id = $1 AND id = $2 AND NOT trashed)"
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn initiative_exists(&mut self, initiative: Uuid) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM initiative WHERE organization_id = $1 AND id = $2)",
        )
        .bind(self.org.0)
        .bind(initiative)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn get_prefs(&mut self, user: UserId) -> AppResult<Option<UserNotificationPrefs>> {
        let row: Option<PrefsRow> = sqlx::query_as(&format!(
            "SELECT {PREFS_COLS} FROM \"user\" WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(prefs_from))
    }

    async fn update_prefs(
        &mut self,
        user: UserId,
        patch: &UserNotificationPrefsPatch,
        now: DateTime<Utc>,
    ) -> AppResult<UserNotificationPrefs> {
        let row: PrefsRow = sqlx::query_as(&format!(
            "UPDATE \"user\" SET email_format = COALESCE($3, email_format), \
             due_date_reminders = COALESCE($4, due_date_reminders), \
             notification_priority_categories = COALESCE($5, notification_priority_categories), \
             notification_settings = notification_settings || COALESCE($6, '{{}}'::jsonb), \
             updated_at = $7, version = version + 1 \
             WHERE organization_id = $1 AND id = $2 RETURNING {PREFS_COLS}"
        ))
        .bind(self.org.0)
        .bind(user.0)
        .bind(&patch.email_format)
        .bind(patch.due_date_reminders)
        .bind(&patch.priority_categories)
        .bind(&patch.settings)
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(prefs_from(row))
    }

    async fn list_channel_preferences(
        &mut self,
        user: UserId,
    ) -> AppResult<Vec<ChannelPreference>> {
        let rows: Vec<(String, String, bool)> = sqlx::query_as(
            "SELECT category, channel, enabled FROM notification_channel_preference \
             WHERE organization_id = $1 AND user_id = $2 AND archived_at IS NULL ORDER BY category, channel",
        )
        .bind(self.org.0)
        .bind(user.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(category, channel, enabled)| ChannelPreference {
                category,
                channel,
                enabled,
            })
            .collect())
    }

    async fn set_channel_preference(
        &mut self,
        user: UserId,
        pref: &ChannelPreference,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO notification_channel_preference (organization_id, user_id, category, channel, \
             enabled, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$6) \
             ON CONFLICT (user_id, category, channel) DO UPDATE SET enabled = EXCLUDED.enabled, \
             updated_at = EXCLUDED.updated_at, version = notification_channel_preference.version + 1",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(&pref.category)
        .bind(&pref.channel)
        .bind(pref.enabled)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn priority_categories(
        &mut self,
        users: &[UserId],
    ) -> AppResult<Vec<(UserId, Vec<String>)>> {
        let rows: Vec<(Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT id, notification_priority_categories FROM \"user\" \
             WHERE organization_id = $1 AND id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(uuids(users))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(u, c)| (UserId(u), c)).collect())
    }
}
