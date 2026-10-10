//! `OutboxRepo` für `PgTx`.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::outbox::{NewSyncAction, SyncActionRow, SyncHead, SyncModelState};
use lineal_app::ports::repos::{NewEvent, OutboxRepo, StoredEvent};
use lineal_app::{AppError, AppResult};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{EventId, OrgId, UserId};
use serde_json::Value;
use uuid::Uuid;

type Row = (Uuid, Uuid, String, Value, i64, DateTime<Utc>, Option<Uuid>);

fn to_stored(
    (id, org, origin, payload, sync_id, created_at, actor): Row,
) -> AppResult<StoredEvent> {
    let origin = origin
        .parse()
        .map_err(|_| AppError::internal("invalid outbox origin"))?;
    let event: DomainEvent = serde_json::from_value(payload)
        .map_err(|e| AppError::internal(format!("undecodable outbox payload: {e}")))?;
    Ok(StoredEvent {
        id: EventId(id),
        org_id: OrgId(org),
        origin,
        actor_id: actor.map(UserId),
        sync_id,
        event,
        created_at,
    })
}

#[async_trait]
impl OutboxRepo for PgTx {
    async fn push(&mut self, event: NewEvent) -> AppResult<EventId> {
        let sync_id = self.last_sync_id_inner().await?;
        let id = EventId::new_v7();
        let payload = serde_json::to_value(&event.event)
            .map_err(|e| AppError::internal(format!("cannot encode event: {e}")))?;
        sqlx::query(
            "INSERT INTO outbox_event (event_id, organization_id, origin, type, payload, sync_id, actor_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id.0)
        .bind(self.org.0)
        .bind(event.origin.as_str())
        .bind(event.event.type_name())
        .bind(payload)
        .bind(sync_id)
        .bind(event.actor_id.map(|a| a.0))
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn claim_undelivered(
        &mut self,
        consumer: &str,
        limit: i64,
    ) -> AppResult<Vec<StoredEvent>> {
        // Bewusst mandantenübergreifend (Relay); nur über `Store::begin_system` aufzurufen.
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT e.event_id, e.organization_id, e.origin, e.payload, e.sync_id, e.created_at, e.actor_id \
             FROM outbox_event e \
             WHERE e.processed_at IS NULL \
               AND NOT EXISTS (SELECT 1 FROM outbox_delivery d \
                               WHERE d.event_id = e.event_id AND d.consumer = $1) \
             ORDER BY e.sync_id \
             LIMIT $2 \
             FOR UPDATE OF e SKIP LOCKED",
        )
        .bind(consumer)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(to_stored).collect()
    }

    async fn mark_delivered(&mut self, consumer: &str, ids: &[EventId]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let uuids: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        sqlx::query(
            "INSERT INTO outbox_delivery (consumer, event_id) \
             SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING",
        )
        .bind(consumer)
        .bind(uuids)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn mark_processed(&mut self, consumers: &[String]) -> AppResult<u64> {
        if consumers.is_empty() {
            return Ok(0);
        }
        let mut names: Vec<String> = consumers.to_vec();
        names.sort();
        names.dedup();
        let res = sqlx::query(
            "UPDATE outbox_event e SET processed_at = now() \
             WHERE e.processed_at IS NULL \
               AND (SELECT count(*) FROM outbox_delivery d \
                    WHERE d.event_id = e.event_id AND d.consumer = ANY($1)) = $2",
        )
        .bind(&names)
        .bind(names.len() as i64)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn sync_append(&mut self, actions: &[NewSyncAction]) -> AppResult<Option<i64>> {
        if actions.is_empty() {
            return Ok(None);
        }
        // Eine Sperre für alle Schreiber, bis zum Commit gehalten: Cursor-IDs werden in
        // Commit-Reihenfolge sichtbar (kein Überspringen durch parallele Konsumenten).
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(SYNC_LOCK_KEY)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let mut last = None;
        for a in actions {
            let id: Option<i64> = sqlx::query_scalar(
                "INSERT INTO sync_log (id, organization_id, sync_id, event_id, n, action, model_name, \
                   model_id, data, sync_groups, origin, actor_id) \
                 SELECT nextval('sync_log_seq'), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
                 WHERE NOT EXISTS (SELECT 1 FROM sync_log WHERE event_id = $3 AND n = $4) \
                 RETURNING id",
            )
            .bind(self.org.0)
            .bind(a.sync_id)
            .bind(a.event_id.0)
            .bind(a.n)
            .bind(&a.action)
            .bind(&a.model_name)
            .bind(a.model_id)
            .bind(&a.data)
            .bind(&a.groups)
            .bind(a.origin.as_str())
            .bind(a.actor_id.map(|u| u.0))
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            if id.is_some() {
                last = id;
            }
        }
        if last.is_some() {
            sqlx::query("SELECT pg_notify('lineal_sync', $1)")
                .bind(self.org.0.to_string())
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
        }
        Ok(last)
    }

    async fn sync_head(&mut self) -> AppResult<SyncHead> {
        let (head, purged): (Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT (SELECT max(id) FROM sync_log WHERE organization_id = $1), \
                    (SELECT purged_up_to FROM sync_log_purged WHERE organization_id = $1)",
        )
        .bind(self.org.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let purged_up_to = purged.unwrap_or(0);
        Ok(SyncHead {
            head: head.unwrap_or(0).max(purged_up_to),
            purged_up_to,
        })
    }

    async fn sync_read(
        &mut self,
        after: i64,
        upto: i64,
        groups: &[String],
        limit: i64,
    ) -> AppResult<Vec<SyncActionRow>> {
        type R = (
            i64,
            i64,
            String,
            String,
            Uuid,
            Value,
            Vec<String>,
            DateTime<Utc>,
        );
        let rows: Vec<R> = sqlx::query_as(
            "SELECT id, sync_id, action, model_name, model_id, data, sync_groups, created_at \
             FROM sync_log \
             WHERE organization_id = $1 AND id > $2 AND id <= $3 AND sync_groups && $4 \
             ORDER BY id LIMIT $5",
        )
        .bind(self.org.0)
        .bind(after)
        .bind(upto)
        .bind(groups)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, sync_id, action, model_name, model_id, data, groups, created_at)| {
                    SyncActionRow {
                        id,
                        sync_id,
                        action,
                        model_name,
                        model_id,
                        data,
                        groups,
                        created_at,
                    }
                },
            )
            .collect())
    }

    async fn sync_last_groups(
        &mut self,
        model_name: &str,
        model_id: Uuid,
    ) -> AppResult<Option<Vec<String>>> {
        // Vereinigung der Gruppen aller Zeilen des letzten (nicht gelöschten) Events zu diesem Objekt
        // (Relationen schreiben je Seite eine Zeile mit eigenen Gruppen).
        let groups: Option<Vec<String>> = sqlx::query_scalar(
            "SELECT array_agg(DISTINCT g) FROM sync_log l, unnest(l.sync_groups) g \
             WHERE l.organization_id = $1 AND l.model_name = $2 AND l.model_id = $3 \
               AND l.event_id = (SELECT event_id FROM sync_log \
                  WHERE organization_id = $1 AND model_name = $2 AND model_id = $3 AND action <> 'D' \
                  ORDER BY id DESC LIMIT 1)",
        )
        .bind(self.org.0)
        .bind(model_name)
        .bind(model_id)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(groups)
    }

    async fn sync_purge(&mut self, before: DateTime<Utc>) -> AppResult<u64> {
        // Mandantenübergreifend: das Wasserzeichen wird je Workspace fortgeschrieben.
        let n: i64 = sqlx::query_scalar(
            "WITH gone AS ( \
               DELETE FROM sync_log WHERE created_at < $1 RETURNING organization_id, id), \
             mark AS ( \
               INSERT INTO sync_log_purged (organization_id, purged_up_to) \
               SELECT organization_id, max(id) FROM gone GROUP BY organization_id \
               ON CONFLICT (organization_id) DO UPDATE \
                 SET purged_up_to = greatest(sync_log_purged.purged_up_to, EXCLUDED.purged_up_to) \
               RETURNING 1) \
             SELECT count(*) FROM gone",
        )
        .bind(before)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn sync_model(
        &mut self,
        model_name: &str,
        model_id: Uuid,
    ) -> AppResult<Option<SyncModelState>> {
        let Some(sql) = model_sql(model_name) else {
            return Ok(None);
        };
        let row: Option<(Value, Vec<Option<String>>)> = sqlx::query_as(&sql)
            .bind(self.org.0)
            .bind(model_id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(row.map(|(data, groups)| SyncModelState {
            data,
            groups: groups.into_iter().flatten().collect(),
        }))
    }
}

/// Schlüssel des Advisory-Locks, der Sync-Log-Schreiber serialisiert.
const SYNC_LOCK_KEY: i64 = 0x4C49_4E53_594E_4331;

/// Sync-Gruppen eines Projekts: alle beteiligten Teams, ohne Team der gesamte Workspace.
fn project_groups(project_id: &str) -> String {
    format!(
        "COALESCE((SELECT array_agg('team:' || pt.team_id::text) FROM project_team pt \
           WHERE pt.project_id = {project_id}), ARRAY['org'])"
    )
}

/// Sync-Gruppen eines Dokuments (Alias `d`): die des Elternobjekts.
fn document_groups() -> String {
    format!(
        "CASE WHEN d.issue_id IS NOT NULL THEN ARRAY['team:' || (SELECT di.team_id FROM issue di \
              WHERE di.id = d.issue_id)::text] \
            WHEN d.project_id IS NOT NULL THEN {} \
            WHEN d.team_id IS NOT NULL THEN ARRAY['team:' || d.team_id::text] \
            ELSE ARRAY['org'] END",
        project_groups("d.project_id")
    )
}

/// Gruppen eines Kommentars (Alias `c` mit `pm`, `pu`, `d`).
fn comment_groups() -> String {
    format!(
        "CASE WHEN c.issue_id IS NOT NULL THEN ARRAY['team:' || (SELECT ci.team_id FROM issue ci \
               WHERE ci.id = c.issue_id)::text] \
             WHEN COALESCE(c.project_id, pm.project_id, pu.project_id) IS NOT NULL THEN {} \
             WHEN c.document_id IS NOT NULL THEN {} \
             ELSE ARRAY['org'] END",
        project_groups("COALESCE(c.project_id, pm.project_id, pu.project_id)"),
        document_groups()
    )
}

const COMMENT_JOINS: &str = "LEFT JOIN project_milestone pm ON pm.id = c.project_milestone_id \
     LEFT JOIN project_update pu ON pu.id = c.project_update_id \
     LEFT JOIN document d ON d.id = c.document_id";

/// SQL (`$1` = Workspace, `$2` = Objekt-ID) je Modell: liefert `(data jsonb, groups text[])`.
/// `data` enthält nur Felder, die die GraphQL-API für berechtigte Leser ebenfalls liefert
/// (keine E-Mails, keine Tokens); große/abgeleitete Felder fehlen, der Client lädt sie nach.
fn model_sql(model: &str) -> Option<String> {
    let sql = match model {
        "Issue" => "SELECT jsonb_build_object( \
              'id', i.id, 'identifier', i.identifier, 'title', i.title, 'description', i.description, \
              'priority', i.priority, 'estimate', i.estimate, 'teamId', i.team_id, \
              'stateId', i.state_id, 'stateName', s.name, 'stateType', i.state_type, \
              'assigneeId', i.assignee_id, 'creatorId', i.creator_id, 'parentId', i.parent_id, \
              'projectId', i.project_id, 'projectMilestoneId', i.project_milestone_id, \
              'cycleId', i.cycle_id, \
              'labelIds', COALESCE((SELECT jsonb_agg(l.id ORDER BY l.name, l.id) FROM issue_to_label il \
                 JOIN issue_label l ON l.id = il.label_id WHERE il.issue_id = i.id), '[]'::jsonb), \
              'dueDate', to_char(i.due_date, 'YYYY-MM-DD'), 'sortOrder', i.sort_order, \
              'startedAt', sync_ts(i.started_at), 'completedAt', sync_ts(i.completed_at), \
              'canceledAt', sync_ts(i.canceled_at), 'archivedAt', sync_ts(i.archived_at), \
              'trashed', i.trashed, 'createdAt', sync_ts(i.created_at), 'updatedAt', sync_ts(i.updated_at)), \
              ARRAY['team:' || i.team_id::text] \
            FROM issue i JOIN workflow_state s ON s.id = i.state_id \
            WHERE i.organization_id = $1 AND i.id = $2"
            .to_owned(),
        "Comment" => format!(
            "SELECT jsonb_build_object( \
              'id', c.id, 'body', c.body, 'userId', c.user_id, 'issueId', c.issue_id, \
              'projectId', c.project_id, 'projectMilestoneId', c.project_milestone_id, \
              'projectUpdateId', c.project_update_id, 'initiativeId', c.initiative_id, \
              'initiativeUpdateId', c.initiative_update_id, 'documentId', c.document_id, \
              'parentId', c.parent_id, 'quotedText', c.quoted_text, 'editedAt', sync_ts(c.edited_at), \
              'resolvedAt', sync_ts(c.resolved_at), 'createdAt', sync_ts(c.created_at), \
              'updatedAt', sync_ts(c.updated_at)), {} \
            FROM comment c {COMMENT_JOINS} WHERE c.organization_id = $1 AND c.id = $2",
            comment_groups()
        ),
        "Attachment" => "SELECT jsonb_build_object('id', a.id, 'issueId', a.issue_id, 'title', a.title, \
              'subtitle', a.subtitle, 'sourceType', a.source_type, 'updatedAt', sync_ts(a.updated_at)), \
              ARRAY['team:' || i.team_id::text] \
            FROM attachment a JOIN issue i ON i.id = a.issue_id \
            WHERE a.organization_id = $1 AND a.id = $2"
            .to_owned(),
        "Reaction" => format!(
            "SELECT jsonb_build_object('id', r.id, 'emoji', r.emoji, 'userId', r.user_id, \
              'issueId', r.issue_id, 'commentId', r.comment_id, 'projectUpdateId', r.project_update_id, \
              'initiativeUpdateId', r.initiative_update_id), \
              CASE WHEN r.issue_id IS NOT NULL THEN ARRAY['team:' || (SELECT ri.team_id FROM issue ri \
                     WHERE ri.id = r.issue_id)::text] \
                   WHEN r.comment_id IS NOT NULL THEN {} \
                   WHEN r.project_update_id IS NOT NULL THEN {} \
                   ELSE ARRAY['org'] END \
            FROM reaction r \
              LEFT JOIN comment c ON c.id = r.comment_id \
              {COMMENT_JOINS} \
              LEFT JOIN project_update ru ON ru.id = r.project_update_id \
            WHERE r.organization_id = $1 AND r.id = $2",
            comment_groups(),
            project_groups("ru.project_id")
        ),
        // Beide Seiten einer Relation: je Seite nur die Gruppen des jeweiligen Issues (die ID des
        // Gegenstücks verrät sonst die Existenz eines Issues in einem unsichtbaren Team).
        "IssueRelation" => "SELECT jsonb_build_object('id', r.id, 'issueId', r.issue_id, \
              'relatedIssueId', r.related_issue_id, 'type', r.type), ARRAY['team:' || i.team_id::text] \
            FROM issue_relation r JOIN issue i ON i.id = r.issue_id \
            WHERE r.organization_id = $1 AND r.id = $2"
            .to_owned(),
        "IssueRelationReverse" => "SELECT jsonb_build_object('id', r.id, 'issueId', r.related_issue_id, \
              'relatedIssueId', r.issue_id, 'type', r.type), ARRAY['team:' || i.team_id::text] \
            FROM issue_relation r JOIN issue i ON i.id = r.related_issue_id \
            WHERE r.organization_id = $1 AND r.id = $2"
            .to_owned(),
        "WorkflowState" => "SELECT jsonb_build_object('id', s.id, 'teamId', s.team_id, 'name', s.name, \
              'type', s.type, 'color', s.color, 'description', s.description, 'position', s.position, \
              'inheritedFromId', s.inherited_from_id, \
              'isDefault', COALESCE(t.default_issue_state_id = s.id, false), \
              'archivedAt', sync_ts(s.archived_at)), ARRAY['team:' || s.team_id::text] \
            FROM workflow_state s JOIN team t ON t.id = s.team_id \
            WHERE s.organization_id = $1 AND s.id = $2"
            .to_owned(),
        "IssueLabel" => "SELECT jsonb_build_object('id', l.id, 'name', l.name, 'color', l.color, \
              'description', l.description, 'teamId', l.team_id, 'parentId', l.parent_id, \
              'isGroup', l.is_group, 'groupType', l.group_type, 'retiredAt', sync_ts(l.retired_at), \
              'archivedAt', sync_ts(l.archived_at), 'createdAt', sync_ts(l.created_at), \
              'updatedAt', sync_ts(l.updated_at)), \
              CASE WHEN l.team_id IS NULL THEN ARRAY['org'] ELSE ARRAY['team:' || l.team_id::text] END \
            FROM issue_label l WHERE l.organization_id = $1 AND l.id = $2"
            .to_owned(),
        "Cycle" => "SELECT jsonb_build_object('id', c.id, 'teamId', c.team_id, 'number', c.number, \
              'name', c.name, 'description', c.description, 'startsAt', sync_ts(c.starts_at), \
              'endsAt', sync_ts(c.ends_at), 'completedAt', sync_ts(c.completed_at), \
              'inheritedFromId', c.inherited_from_id, 'archivedAt', sync_ts(c.archived_at), \
              'createdAt', sync_ts(c.created_at), 'updatedAt', sync_ts(c.updated_at)), \
              ARRAY['team:' || c.team_id::text] \
            FROM cycle c WHERE c.organization_id = $1 AND c.id = $2"
            .to_owned(),
        "Project" => format!(
            "SELECT jsonb_build_object('id', p.id, 'name', p.name, 'color', p.color, \
              'archivedAt', sync_ts(p.archived_at), 'updatedAt', sync_ts(p.updated_at)), {} \
            FROM project p WHERE p.organization_id = $1 AND p.id = $2",
            project_groups("p.id")
        ),
        "ProjectMilestone" => format!(
            "SELECT jsonb_build_object('id', m.id, 'projectId', m.project_id, 'name', m.name, \
              'targetDate', to_char(m.target_date, 'YYYY-MM-DD'), 'updatedAt', sync_ts(m.updated_at)), {} \
            FROM project_milestone m WHERE m.organization_id = $1 AND m.id = $2",
            project_groups("m.project_id")
        ),
        "ProjectUpdate" => format!(
            "SELECT jsonb_build_object('id', u.id, 'projectId', u.project_id, 'userId', u.user_id, \
              'health', u.health, 'createdAt', sync_ts(u.created_at), 'updatedAt', sync_ts(u.updated_at)), {} \
            FROM project_update u WHERE u.organization_id = $1 AND u.id = $2",
            project_groups("u.project_id")
        ),
        "Initiative" => "SELECT jsonb_build_object('id', n.id, 'name', n.name, \
              'archivedAt', sync_ts(n.archived_at), 'updatedAt', sync_ts(n.updated_at)), ARRAY['org'] \
            FROM initiative n WHERE n.organization_id = $1 AND n.id = $2"
            .to_owned(),
        "InitiativeUpdate" => "SELECT jsonb_build_object('id', u.id, 'initiativeId', u.initiative_id, \
              'userId', u.user_id, 'updatedAt', sync_ts(u.updated_at)), ARRAY['org'] \
            FROM initiative_update u WHERE u.organization_id = $1 AND u.id = $2"
            .to_owned(),
        "Document" => format!(
            "SELECT jsonb_build_object('id', d.id, 'title', d.title, 'icon', d.icon, 'color', d.color, \
              'projectId', d.project_id, 'issueId', d.issue_id, 'initiativeId', d.initiative_id, \
              'cycleId', d.cycle_id, 'teamId', d.team_id, 'trashed', d.trashed, \
              'updatedAt', sync_ts(d.updated_at)), {} \
            FROM document d WHERE d.organization_id = $1 AND d.id = $2",
            document_groups()
        ),
        "Team" => "SELECT jsonb_build_object('id', t.id, 'key', t.key, 'name', t.name, \
              'parentId', t.parent_id, 'updatedAt', sync_ts(t.updated_at)), ARRAY['team:' || t.id::text] \
            FROM team t WHERE t.organization_id = $1 AND t.id = $2"
            .to_owned(),
        "Template" => "SELECT jsonb_build_object('id', t.id, 'type', t.type, 'name', t.name, \
              'teamId', t.team_id, 'updatedAt', sync_ts(t.updated_at)), \
              CASE WHEN t.team_id IS NULL THEN ARRAY['org'] ELSE ARRAY['team:' || t.team_id::text] END \
            FROM template t WHERE t.organization_id = $1 AND t.id = $2"
            .to_owned(),
        "CustomView" => format!(
            "SELECT jsonb_build_object('id', v.id, 'name', v.name, 'ownerId', v.owner_id, \
              'shared', v.shared, 'teamId', v.team_id, 'updatedAt', sync_ts(v.updated_at)), \
              CASE WHEN NOT v.shared THEN ARRAY['user:' || v.owner_id::text] \
                   WHEN v.team_id IS NOT NULL THEN ARRAY['team:' || v.team_id::text] \
                   WHEN v.project_id IS NOT NULL THEN {} \
                   ELSE ARRAY['org'] END \
            FROM custom_view v WHERE v.organization_id = $1 AND v.id = $2",
            project_groups("v.project_id")
        ),
        "User" => "SELECT jsonb_build_object('id', u.id, 'name', u.name, 'displayName', u.display_name, \
              'updatedAt', sync_ts(u.updated_at)), ARRAY['org'] \
            FROM \"user\" u WHERE u.organization_id = $1 AND u.id = $2"
            .to_owned(),
        "Organization" => "SELECT jsonb_build_object('id', o.id, 'name', o.name), ARRAY['org'] \
            FROM organization o WHERE o.id = $1 AND o.id = $2"
            .to_owned(),
        _ => return None,
    };
    Some(sql)
}
