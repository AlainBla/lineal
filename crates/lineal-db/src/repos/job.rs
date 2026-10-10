//! `JobRepo` für `PgTx` (Queue mit `FOR UPDATE SKIP LOCKED`).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::{Job, JobRepo, NewJob};
use lineal_domain::ids::{JobId, OrgId};
use serde_json::Value;
use uuid::Uuid;

#[async_trait]
impl JobRepo for PgTx {
    async fn enqueue(&mut self, job: NewJob) -> AppResult<Option<JobId>> {
        let id = JobId::new_v7();
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO job (id, organization_id, kind, payload, run_at, max_attempts, unique_key) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(id.0)
        .bind(job.org_id.map(|o| o.0))
        .bind(&job.kind)
        .bind(&job.payload)
        .bind(job.run_at)
        .bind(job.max_attempts)
        .bind(&job.unique_key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(inserted.map(JobId))
    }

    async fn claim(&mut self, worker: &str, limit: i64, now: DateTime<Utc>) -> AppResult<Vec<Job>> {
        let rows: Vec<(Uuid, Option<Uuid>, String, Value, i32, i32)> = sqlx::query_as(
            "UPDATE job SET status = 'running', locked_by = $1, locked_at = $3, attempts = attempts + 1 \
             WHERE id IN ( \
                 SELECT id FROM job \
                 WHERE (status = 'queued' AND run_at <= $3) \
                    OR (status = 'running' AND locked_at < $3 - interval '15 minutes' \
                        AND attempts < max_attempts) \
                 ORDER BY run_at \
                 LIMIT $2 \
                 FOR UPDATE SKIP LOCKED) \
             RETURNING id, organization_id, kind, payload, attempts, max_attempts",
        )
        .bind(worker)
        .bind(limit)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, org, kind, payload, attempts, max_attempts)| Job {
                id: JobId(id),
                org_id: org.map(OrgId),
                kind,
                payload,
                attempts,
                max_attempts,
            })
            .collect())
    }

    async fn complete(&mut self, id: JobId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE job SET status = 'done', completed_at = $2, locked_by = NULL, locked_at = NULL \
             WHERE id = $1",
        )
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn fail(
        &mut self,
        id: JobId,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE job SET last_error = $2, locked_by = NULL, locked_at = NULL, \
                 status = CASE WHEN $3::timestamptz IS NOT NULL AND attempts < max_attempts \
                               THEN 'queued' ELSE 'failed' END, \
                 run_at = COALESCE($3::timestamptz, run_at) \
             WHERE id = $1",
        )
        .bind(id.0)
        .bind(error)
        .bind(retry_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
