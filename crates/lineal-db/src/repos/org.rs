//! `OrgRepo` für `PgTx`.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::OrgRepo;
use lineal_app::ports::repos::org::{
    NewOrg, NewProjectStatus, NewWorkspaceLabel, Org, OrgPatch,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::OrgId;
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

/// Bildet Unique-Verletzungen (SQLSTATE 23505) auf `conflict` ab, alles andere auf `internal`.
pub(crate) fn conflict_or_internal(e: sqlx::Error, message: &str) -> AppError {
    let unique = e
        .as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505");
    if unique {
        AppError::conflict(message)
    } else {
        db_err(e)
    }
}

#[derive(sqlx::FromRow)]
struct OrgRow {
    id: Uuid,
    name: String,
    url_key: String,
    previous_url_keys: Vec<String>,
    logo_url: Option<String>,
    fiscal_year_start_month: i32,
    initiatives_enabled: bool,
    security_settings: Value,
    limits: Value,
    deletion_requested_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const ORG_COLS: &str = "id, name, url_key, previous_url_keys, logo_url, fiscal_year_start_month, \
    initiatives_enabled, security_settings, limits, deletion_requested_at, version, created_at, updated_at";

impl From<OrgRow> for Org {
    fn from(r: OrgRow) -> Self {
        Org {
            id: OrgId(r.id),
            name: r.name,
            url_key: r.url_key,
            previous_url_keys: r.previous_url_keys,
            logo_url: r.logo_url,
            fiscal_year_start_month: r.fiscal_year_start_month,
            initiatives_enabled: r.initiatives_enabled,
            security_settings: r.security_settings,
            limits: r.limits,
            deletion_requested_at: r.deletion_requested_at,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[async_trait]
impl OrgRepo for PgTx {
    async fn insert(&mut self, org: &NewOrg) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO organization (id, name, url_key, security_settings, limits, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6)",
        )
        .bind(org.id.0)
        .bind(&org.name)
        .bind(&org.url_key)
        .bind(&org.security_settings)
        .bind(&org.limits)
        .bind(org.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "url key already in use"))?;
        Ok(())
    }

    async fn get(&mut self) -> AppResult<Option<Org>> {
        let row = sqlx::query_as::<_, OrgRow>(&format!(
            "SELECT {ORG_COLS} FROM organization WHERE id = $1"
        ))
        .bind(self.org.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Org::from))
    }

    async fn url_key_taken(&mut self, url_key: &str) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM organization WHERE url_key = $1 OR $1 = ANY (previous_url_keys))",
        )
        .bind(url_key)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn update(&mut self, patch: &OrgPatch) -> AppResult<Org> {
        let now = patch.now.unwrap_or_else(Utc::now);
        let mut qb: QueryBuilder<'_, Postgres> =
            QueryBuilder::new("UPDATE organization SET version = version + 1, updated_at = ");
        qb.push_bind(now);
        if let Some(v) = &patch.name {
            qb.push(", name = ").push_bind(v.clone());
        }
        if let Some(v) = &patch.url_key {
            // Der alte Key wandert in previous_url_keys (die letzten 3 bleiben).
            qb.push(
                ", previous_url_keys = (SELECT COALESCE(array_agg(k ORDER BY n), '{}') FROM ( \
                 SELECT k, n FROM unnest(array_append(previous_url_keys, url_key)) WITH ORDINALITY AS t(k, n) \
                 ORDER BY n DESC LIMIT 3) s)",
            );
            qb.push(", url_key = ").push_bind(v.clone());
        }
        if let Some(v) = &patch.logo_url {
            qb.push(", logo_url = ").push_bind(v.clone());
        }
        if let Some(v) = patch.fiscal_year_start_month {
            qb.push(", fiscal_year_start_month = ").push_bind(v);
        }
        if let Some(v) = patch.initiatives_enabled {
            qb.push(", initiatives_enabled = ").push_bind(v);
        }
        if let Some(v) = &patch.security_settings {
            qb.push(", security_settings = ").push_bind(v.clone());
        }
        if let Some(v) = &patch.limits {
            qb.push(", limits = ").push_bind(v.clone());
        }
        qb.push(" WHERE id = ").push_bind(self.org.0);
        qb.push(format!(" RETURNING {ORG_COLS}"));
        let row = qb
            .build_query_as::<OrgRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        row.map(Org::from)
            .ok_or_else(|| AppError::not_found("Could not find referenced Workspace."))
    }

    async fn insert_project_status(&mut self, s: &NewProjectStatus) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project_status (id, organization_id, name, type, color, position, indefinite, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(s.id)
        .bind(self.org.0)
        .bind(&s.name)
        .bind(s.status_type.as_str())
        .bind(&s.color)
        .bind(s.position)
        .bind(s.indefinite)
        .bind(s.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn insert_workspace_label(&mut self, l: &NewWorkspaceLabel) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_label (id, organization_id, name, color, creator_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6)",
        )
        .bind(l.id)
        .bind(self.org.0)
        .bind(&l.name)
        .bind(&l.color)
        .bind(l.creator_id)
        .bind(l.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
