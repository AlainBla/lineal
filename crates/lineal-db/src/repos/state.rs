//! `WorkflowStateRepo` für `PgTx`.

use super::org::conflict_or_internal;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::WorkflowStateRepo;
use lineal_app::ports::repos::state::{NewState, StatePatch, WorkflowState};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::StateType;
use lineal_domain::ids::{OrgId, StateId, TeamId, UserId};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct StateRow {
    id: Uuid,
    organization_id: Uuid,
    team_id: Uuid,
    name: String,
    #[sqlx(rename = "type")]
    state_type: String,
    color: String,
    description: Option<String>,
    position: f64,
    inherited_from_id: Option<Uuid>,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const STATE_COLS: &str = "id, organization_id, team_id, name, type, color, description, position, \
    inherited_from_id, archived_at, version, created_at, updated_at";

/// Reihenfolge der Kategorien (docs/03 §1.1) als SQL-Ausdruck.
const TYPE_ORDER: &str = "CASE type WHEN 'triage' THEN 0 WHEN 'backlog' THEN 1 WHEN 'unstarted' THEN 2 \
    WHEN 'started' THEN 3 WHEN 'completed' THEN 4 WHEN 'canceled' THEN 5 ELSE 6 END";

impl StateRow {
    fn into_state(self) -> AppResult<WorkflowState> {
        Ok(WorkflowState {
            id: StateId(self.id),
            org_id: OrgId(self.organization_id),
            team_id: TeamId(self.team_id),
            name: self.name,
            state_type: self
                .state_type
                .parse()
                .map_err(|_| AppError::internal("invalid state type in database"))?,
            color: self.color,
            description: self.description,
            position: self.position,
            inherited_from_id: self.inherited_from_id.map(StateId),
            archived_at: self.archived_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[async_trait]
impl WorkflowStateRepo for PgTx {
    async fn insert(&mut self, s: &NewState) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO workflow_state (id, organization_id, team_id, name, type, color, description, position, \
             inherited_from_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
        )
        .bind(s.id.0)
        .bind(self.org.0)
        .bind(s.team_id.0)
        .bind(&s.name)
        .bind(s.state_type.as_str())
        .bind(&s.color)
        .bind(&s.description)
        .bind(s.position)
        .bind(s.inherited_from_id.map(|i| i.0))
        .bind(s.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "workflow state name already in use"))?;
        Ok(())
    }

    async fn get(&mut self, id: StateId) -> AppResult<Option<WorkflowState>> {
        let row = sqlx::query_as::<_, StateRow>(&format!(
            "SELECT {STATE_COLS} FROM workflow_state WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(StateRow::into_state).transpose()
    }

    async fn list_for_team(
        &mut self,
        team: TeamId,
        include_archived: bool,
    ) -> AppResult<Vec<WorkflowState>> {
        let rows = sqlx::query_as::<_, StateRow>(&format!(
            "SELECT {STATE_COLS} FROM workflow_state \
             WHERE organization_id = $1 AND team_id = $2 AND ($3 OR archived_at IS NULL) \
             ORDER BY {TYPE_ORDER}, position, created_at, id"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .bind(include_archived)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(StateRow::into_state).collect()
    }

    async fn update(
        &mut self,
        id: StateId,
        p: &StatePatch,
        now: DateTime<Utc>,
    ) -> AppResult<WorkflowState> {
        let mut qb: QueryBuilder<'_, Postgres> =
            QueryBuilder::new("UPDATE workflow_state SET version = version + 1, updated_at = ");
        qb.push_bind(now);
        if let Some(v) = &p.name {
            qb.push(", name = ").push_bind(v.clone());
        }
        if let Some(v) = &p.color {
            qb.push(", color = ").push_bind(v.clone());
        }
        if let Some(v) = &p.description {
            qb.push(", description = ").push_bind(v.clone());
        }
        if let Some(v) = p.position {
            qb.push(", position = ").push_bind(v);
        }
        if let Some(v) = p.inherited_from {
            qb.push(", inherited_from_id = ").push_bind(v.map(|s| s.0));
        }
        qb.push(" WHERE organization_id = ").push_bind(self.org.0);
        qb.push(" AND id = ").push_bind(id.0);
        qb.push(format!(" RETURNING {STATE_COLS}"));
        let row = qb
            .build_query_as::<StateRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|e| conflict_or_internal(e, "workflow state name already in use"))?;
        row.ok_or_else(|| AppError::not_found("Could not find referenced WorkflowState."))?
            .into_state()
    }

    async fn archive(&mut self, id: StateId, now: DateTime<Utc>) -> AppResult<()> {
        let res = sqlx::query(
            "UPDATE workflow_state SET archived_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced WorkflowState."));
        }
        Ok(())
    }

    async fn count_active_of_type(
        &mut self,
        team: TeamId,
        state_type: StateType,
    ) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM workflow_state \
             WHERE organization_id = $1 AND team_id = $2 AND type = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(state_type.as_str())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn count_issues(&mut self, id: StateId) -> AppResult<i64> {
        sqlx::query_scalar("SELECT count(*) FROM issue WHERE organization_id = $1 AND state_id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)
    }

    async fn migrate_issues(
        &mut self,
        from: StateId,
        to: StateId,
        actor: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        // History zuerst (liest den alten Status), dann die Issues umhängen.
        sqlx::query(
            "INSERT INTO issue_history (id, organization_id, issue_id, actor_id, source, changes, created_at) \
             SELECT gen_random_uuid(), i.organization_id, i.id, $4, 'system', \
                    jsonb_build_object('stateId', jsonb_build_object('from', $2::text, 'to', $3::text)), $5 \
             FROM issue i WHERE i.organization_id = $1 AND i.state_id = $2",
        )
        .bind(self.org.0)
        .bind(from.0)
        .bind(to.0)
        .bind(actor.map(|a| a.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        // Kategorie-Zeitstempel nach docs/03 §1.6 (nur bei Kategoriewechsel).
        let res = sqlx::query(
            "UPDATE issue i SET state_id = $3, state_entered_at = $4, version = i.version + 1, updated_at = $4, \
               started_at = CASE \
                 WHEN o.type = t.type THEN i.started_at \
                 WHEN t.type IN ('triage', 'backlog', 'unstarted') THEN NULL \
                 WHEN t.type IN ('started', 'completed') THEN COALESCE(i.started_at, $4) \
                 ELSE i.started_at END, \
               completed_at = CASE WHEN o.type = t.type THEN i.completed_at \
                 WHEN t.type = 'completed' THEN $4 ELSE NULL END, \
               canceled_at = CASE WHEN o.type = t.type THEN i.canceled_at \
                 WHEN t.type IN ('canceled', 'duplicate') THEN $4 ELSE NULL END \
             FROM workflow_state o, workflow_state t \
             WHERE i.organization_id = $1 AND i.state_id = $2 \
               AND o.organization_id = $1 AND o.id = $2 AND t.organization_id = $1 AND t.id = $3",
        )
        .bind(self.org.0)
        .bind(from.0)
        .bind(to.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn list_copies_of(&mut self, source: StateId) -> AppResult<Vec<WorkflowState>> {
        let rows = sqlx::query_as::<_, StateRow>(&format!(
            "SELECT {STATE_COLS} FROM workflow_state \
             WHERE organization_id = $1 AND inherited_from_id = $2 AND archived_at IS NULL ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(source.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(StateRow::into_state).collect()
    }

    async fn find_by_name(
        &mut self,
        team: TeamId,
        name: &str,
    ) -> AppResult<Option<WorkflowState>> {
        let row = sqlx::query_as::<_, StateRow>(&format!(
            "SELECT {STATE_COLS} FROM workflow_state \
             WHERE organization_id = $1 AND team_id = $2 AND lower(name) = lower($3) AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .bind(name)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(StateRow::into_state).transpose()
    }
}
