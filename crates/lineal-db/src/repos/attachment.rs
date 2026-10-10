//! `AttachmentRepo` für `PgTx` (WP09): Anhänge, Upload-Reservierungen, Verlaufseintrag.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::AttachmentRepo;
use lineal_app::ports::repos::attachment::{Attachment, IssueRef, NewLink, NewUpload, Reservation};
use lineal_domain::ids::*;
use serde_json::Value;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    id: Uuid,
    issue_id: Uuid,
    url: String,
    title: String,
    subtitle: Option<String>,
    source_type: Option<String>,
    content_type: Option<String>,
    size: Option<i64>,
    storage_key: Option<String>,
    metadata: Value,
    creator_id: Option<Uuid>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
}

const COLS: &str = "id, issue_id, url, title, subtitle, source_type, content_type, size, \
    storage_key, metadata, creator_id, version, created_at, updated_at, archived_at";

impl From<AttachmentRow> for Attachment {
    fn from(r: AttachmentRow) -> Self {
        Self {
            id: AttachmentId(r.id),
            issue_id: IssueId(r.issue_id),
            url: r.url,
            title: r.title,
            subtitle: r.subtitle,
            source_type: r.source_type,
            content_type: r.content_type,
            size: r.size,
            storage_key: r.storage_key,
            metadata: r.metadata,
            creator_id: r.creator_id.map(UserId),
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
            archived_at: r.archived_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ReservationRow {
    id: Uuid,
    issue_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    storage_key: String,
    filename: String,
    content_type: String,
    size: i64,
    title: Option<String>,
    subtitle: Option<String>,
    expires_at: DateTime<Utc>,
    finalized_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[async_trait]
impl AttachmentRepo for PgTx {
    async fn find_issue(&mut self, reference: &str) -> AppResult<Option<IssueRef>> {
        match self.find_issue_id(reference).await? {
            Some(id) => self.issue_ref(IssueId(id)).await,
            None => Ok(None),
        }
    }

    async fn issue_ref(&mut self, id: IssueId) -> AppResult<Option<IssueRef>> {
        let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT id, team_id, identifier FROM issue \
             WHERE organization_id = $1 AND id = $2 AND NOT trashed",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(id, team, identifier)| IssueRef {
            id: IssueId(id),
            team_id: TeamId(team),
            identifier,
        }))
    }

    async fn upsert_link(&mut self, l: &NewLink) -> AppResult<(Attachment, bool)> {
        let existing: Option<(Uuid, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT id, archived_at FROM attachment \
             WHERE organization_id = $1 AND issue_id = $2 AND url = $3 FOR UPDATE",
        )
        .bind(self.org.0)
        .bind(l.issue_id.0)
        .bind(&l.url)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let Some((id, archived_at)) = existing else {
            let row = sqlx::query_as::<_, AttachmentRow>(&format!(
                "INSERT INTO attachment (id, organization_id, issue_id, url, title, subtitle, \
                 source_type, metadata, creator_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'url', COALESCE($7, '{{}}'::jsonb), $8, $9, $9) \
                 RETURNING {COLS}"
            ))
            .bind(l.id.0)
            .bind(self.org.0)
            .bind(l.issue_id.0)
            .bind(&l.url)
            .bind(&l.title)
            .bind(&l.subtitle)
            .bind(&l.metadata)
            .bind(l.creator_id.map(|u| u.0))
            .bind(l.now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)?;
            return Ok((row.into(), true));
        };
        let row = sqlx::query_as::<_, AttachmentRow>(&format!(
            "UPDATE attachment SET title = $3, subtitle = $4, \
             metadata = COALESCE($5, metadata), archived_at = NULL, \
             creator_id = CASE WHEN $6 THEN $7 ELSE creator_id END, \
             updated_at = $8, version = version + 1 \
             WHERE organization_id = $1 AND id = $2 RETURNING {COLS}"
        ))
        .bind(self.org.0)
        .bind(id)
        .bind(&l.title)
        .bind(&l.subtitle)
        .bind(&l.metadata)
        .bind(archived_at.is_some())
        .bind(l.creator_id.map(|u| u.0))
        .bind(l.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok((row.into(), archived_at.is_some()))
    }

    async fn insert_upload(&mut self, u: &NewUpload) -> AppResult<Attachment> {
        let row = sqlx::query_as::<_, AttachmentRow>(&format!(
            "INSERT INTO attachment (id, organization_id, issue_id, url, title, subtitle, source_type, \
             content_type, size, storage_key, creator_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'upload', $7, $8, $9, $10, $11, $11) RETURNING {COLS}"
        ))
        .bind(u.id.0)
        .bind(self.org.0)
        .bind(u.issue_id.0)
        .bind(&u.url)
        .bind(&u.title)
        .bind(&u.subtitle)
        .bind(&u.content_type)
        .bind(u.size)
        .bind(&u.storage_key)
        .bind(u.creator_id.map(|c| c.0))
        .bind(u.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.into())
    }

    async fn get(&mut self, id: AttachmentId) -> AppResult<Option<Attachment>> {
        let row = sqlx::query_as::<_, AttachmentRow>(&format!(
            "SELECT {COLS} FROM attachment WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn find_by_storage_key(&mut self, key: &str) -> AppResult<Option<Attachment>> {
        let row = sqlx::query_as::<_, AttachmentRow>(&format!(
            "SELECT {COLS} FROM attachment WHERE organization_id = $1 AND storage_key = $2 \
             AND archived_at IS NULL ORDER BY created_at LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn list_for_issue(&mut self, issue: IssueId) -> AppResult<Vec<Attachment>> {
        let rows = sqlx::query_as::<_, AttachmentRow>(&format!(
            "SELECT {COLS} FROM attachment WHERE organization_id = $1 AND issue_id = $2 \
             AND archived_at IS NULL ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn archive(&mut self, id: AttachmentId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachment SET archived_at = $3, updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn add_history(
        &mut self,
        issue: IssueId,
        actor: Option<UserId>,
        attachment: AttachmentId,
        flag: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_history (id, organization_id, issue_id, actor_id, source, attachment_id, \
             flags, created_at) VALUES ($1, $2, $3, $4, 'user', $5, ARRAY[$6]::text[], $7)",
        )
        .bind(Uuid::now_v7())
        .bind(self.org.0)
        .bind(issue.0)
        .bind(actor.map(|a| a.0))
        .bind(attachment.0)
        .bind(flag)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn insert_reservation(&mut self, r: &Reservation) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO upload_reservation (id, organization_id, issue_id, creator_id, storage_key, \
             filename, content_type, size, title, subtitle, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(r.id)
        .bind(self.org.0)
        .bind(r.issue_id.map(|i| i.0))
        .bind(r.creator_id.map(|u| u.0))
        .bind(&r.storage_key)
        .bind(&r.filename)
        .bind(&r.content_type)
        .bind(r.size)
        .bind(&r.title)
        .bind(&r.subtitle)
        .bind(r.expires_at)
        .bind(r.created_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn reservation_by_key(&mut self, key: &str) -> AppResult<Option<Reservation>> {
        let row: Option<ReservationRow> = sqlx::query_as(
            "SELECT id, issue_id, creator_id, storage_key, filename, content_type, size, title, subtitle, \
             expires_at, finalized_at, created_at FROM upload_reservation \
             WHERE organization_id = $1 AND storage_key = $2",
        )
        .bind(self.org.0)
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| Reservation {
            id: r.id,
            issue_id: r.issue_id.map(IssueId),
            creator_id: r.creator_id.map(UserId),
            storage_key: r.storage_key,
            filename: r.filename,
            content_type: r.content_type,
            size: r.size,
            title: r.title,
            subtitle: r.subtitle,
            expires_at: r.expires_at,
            finalized_at: r.finalized_at,
            created_at: r.created_at,
        }))
    }

    async fn finalize_reservation(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<bool> {
        let n = sqlx::query(
            "UPDATE upload_reservation SET finalized_at = $3 \
             WHERE organization_id = $1 AND id = $2 AND finalized_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(n == 1)
    }

    async fn storage_keys_for_issue(&mut self, issue: IssueId) -> AppResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT storage_key FROM attachment WHERE organization_id = $1 AND issue_id = $2 \
               AND storage_key IS NOT NULL \
             UNION SELECT storage_key FROM upload_reservation WHERE organization_id = $1 AND issue_id = $2",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn storage_keys_for_team(&mut self, team: TeamId) -> AppResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT a.storage_key FROM attachment a JOIN issue i \
               ON i.organization_id = a.organization_id AND i.id = a.issue_id \
             WHERE a.organization_id = $1 AND i.team_id = $2 AND a.storage_key IS NOT NULL \
             UNION SELECT r.storage_key FROM upload_reservation r JOIN issue i \
               ON i.organization_id = r.organization_id AND i.id = r.issue_id \
             WHERE r.organization_id = $1 AND i.team_id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn unreferenced_storage_keys(&mut self, keys: &[String]) -> AppResult<Vec<String>> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        // Objektschlüssel sind global; auch Verweise anderer Workspaces zählen.
        sqlx::query_scalar(
            "SELECT k FROM unnest($1::text[]) AS k \
             WHERE NOT EXISTS (SELECT 1 FROM attachment WHERE storage_key = k) \
               AND NOT EXISTS (SELECT 1 FROM upload_reservation WHERE storage_key = k)",
        )
        .bind(keys)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn list_expired_reservations(
        &mut self,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<(OrgId, Reservation)>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            organization_id: Uuid,
            #[sqlx(flatten)]
            r: ReservationRow,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT organization_id, id, issue_id, creator_id, storage_key, filename, content_type, size, \
             title, subtitle, expires_at, finalized_at, created_at FROM upload_reservation \
             WHERE finalized_at IS NULL AND expires_at < $1 ORDER BY expires_at LIMIT $2",
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let r = row.r;
                (
                    OrgId(row.organization_id),
                    Reservation {
                        id: r.id,
                        issue_id: r.issue_id.map(IssueId),
                        creator_id: r.creator_id.map(UserId),
                        storage_key: r.storage_key,
                        filename: r.filename,
                        content_type: r.content_type,
                        size: r.size,
                        title: r.title,
                        subtitle: r.subtitle,
                        expires_at: r.expires_at,
                        finalized_at: r.finalized_at,
                        created_at: r.created_at,
                    },
                )
            })
            .collect())
    }

    async fn delete_reservations(&mut self, ids: &[Uuid]) -> AppResult<()> {
        sqlx::query("DELETE FROM upload_reservation WHERE organization_id = $1 AND id = ANY($2)")
            .bind(self.org.0)
            .bind(ids)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}
