//! `AuditRepo` für `PgTx`. Alle Abfragen filtern auf `organization_id` der Transaktion; die
//! Aufbewahrung läuft über die SQL-Funktion `audit_log_purge_expired` (siehe Migration 2500).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::audit::{
    AuditCursor, AuditEntry, AuditFilter, AuditRepo, NewAuditEntry,
};
use lineal_domain::ids::{OrgId, UserId};
use serde_json::Value;
use uuid::Uuid;

type Row = (
    Uuid,
    Uuid,
    DateTime<Utc>,
    Option<Uuid>,
    String,
    Option<String>,
    String,
    String,
    Option<Uuid>,
    Value,
    Option<String>,
    String,
);

fn to_entry(r: Row) -> AuditEntry {
    let (
        id,
        organization_id,
        occurred_at,
        actor_user_id,
        actor_kind,
        credential_kind,
        action,
        target_type,
        target_id,
        details,
        ip,
        request_id,
    ) = r;
    AuditEntry {
        id,
        organization_id: OrgId(organization_id),
        occurred_at,
        actor_user_id: actor_user_id.map(UserId),
        actor_kind,
        credential_kind,
        action,
        target_type,
        target_id,
        details,
        ip,
        request_id,
    }
}

#[async_trait]
impl AuditRepo for PgTx {
    async fn append(&mut self, e: &NewAuditEntry) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO audit_log (id, organization_id, occurred_at, actor_user_id, actor_kind, \
             credential_kind, action, target_type, target_id, details, ip, request_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::inet, $12)",
        )
        .bind(e.id)
        .bind(e.organization_id.0)
        .bind(e.occurred_at)
        .bind(e.actor_user_id.map(|u| u.0))
        .bind(e.actor_kind.as_str())
        .bind(e.credential_kind)
        .bind(&e.action)
        .bind(&e.target_type)
        .bind(e.target_id)
        .bind(&e.details)
        .bind(e.ip.map(|i| i.to_string()))
        .bind(&e.request_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list(
        &mut self,
        filter: &AuditFilter,
        cursor: Option<AuditCursor>,
        limit: i64,
    ) -> AppResult<Vec<AuditEntry>> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, organization_id, occurred_at, actor_user_id, actor_kind, credential_kind, \
             action, target_type, target_id, details, host(ip), request_id \
             FROM audit_log \
             WHERE organization_id = $1 \
               AND ($2::text IS NULL OR action = $2) \
               AND ($3::uuid IS NULL OR actor_user_id = $3) \
               AND ($4::timestamptz IS NULL OR occurred_at >= $4) \
               AND ($5::timestamptz IS NULL OR occurred_at < $5) \
               AND ($6::timestamptz IS NULL OR (occurred_at, id) < ($6, $7::uuid)) \
             ORDER BY occurred_at DESC, id DESC \
             LIMIT $8",
        )
        .bind(self.org.0)
        .bind(filter.action.as_deref())
        .bind(filter.actor_user_id.map(|u| u.0))
        .bind(filter.since)
        .bind(filter.until)
        .bind(cursor.map(|c| c.occurred_at))
        .bind(cursor.map(|c| c.id))
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(to_entry).collect())
    }

    async fn purge_expired(&mut self, now: DateTime<Utc>) -> AppResult<u64> {
        let n: i64 = sqlx::query_scalar("SELECT audit_log_purge_expired($1)")
            .bind(now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
