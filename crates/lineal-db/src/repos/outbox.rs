//! `OutboxRepo` für `PgTx`.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::{NewEvent, OutboxRepo, StoredEvent};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::{EventId, OrgId};
use lineal_domain::events::DomainEvent;
use serde_json::Value;
use uuid::Uuid;

type Row = (Uuid, Uuid, String, Value, i64, DateTime<Utc>);

fn to_stored((id, org, origin, payload, sync_id, created_at): Row) -> AppResult<StoredEvent> {
    let origin = origin.parse().map_err(|_| AppError::internal("invalid outbox origin"))?;
    let event: DomainEvent = serde_json::from_value(payload)
        .map_err(|e| AppError::internal(format!("undecodable outbox payload: {e}")))?;
    Ok(StoredEvent { id: EventId(id), org_id: OrgId(org), origin, sync_id, event, created_at })
}

#[async_trait]
impl OutboxRepo for PgTx {
    async fn push(&mut self, event: NewEvent) -> AppResult<EventId> {
        let sync_id = self.last_sync_id_inner().await?;
        let id = EventId::new_v7();
        let payload = serde_json::to_value(&event.event)
            .map_err(|e| AppError::internal(format!("cannot encode event: {e}")))?;
        sqlx::query(
            "INSERT INTO outbox_event (event_id, organization_id, origin, type, payload, sync_id) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id.0)
        .bind(self.org.0)
        .bind(event.origin.as_str())
        .bind(event.event.type_name())
        .bind(payload)
        .bind(sync_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn claim_undelivered(&mut self, consumer: &str, limit: i64) -> AppResult<Vec<StoredEvent>> {
        // Bewusst mandantenübergreifend (Relay); nur über `Store::begin_system` aufzurufen.
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT e.event_id, e.organization_id, e.origin, e.payload, e.sync_id, e.created_at \
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
}
