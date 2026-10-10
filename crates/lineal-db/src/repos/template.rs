//! `TemplateRepo` für `PgTx` (WP10).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::TemplateRepo;
use lineal_app::ports::repos::template::{DefaultSlot, NewTemplate, TeamDefaults, Template};
use lineal_domain::enums::TemplateType;
use lineal_domain::ids::*;
use serde_json::Value;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    #[sqlx(rename = "type")]
    template_type: String,
    name: String,
    description: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    team_id: Option<Uuid>,
    template_data: Value,
    sort_order: f64,
    inherited_from_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    last_updated_by_id: Option<Uuid>,
    last_applied_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
}

const COLS: &str = "id, type, name, description, icon, color, team_id, template_data, sort_order, \
    inherited_from_id, creator_id, last_updated_by_id, last_applied_at, version, created_at, \
    updated_at, archived_at";

impl Row {
    fn into_template(self) -> AppResult<Template> {
        Ok(Template {
            id: TemplateId(self.id),
            template_type: self.template_type.parse::<TemplateType>().map_err(|_| {
                lineal_app::AppError::internal(format!(
                    "unknown template type `{}`",
                    self.template_type
                ))
            })?,
            name: self.name,
            description: self.description,
            icon: self.icon,
            color: self.color,
            team_id: self.team_id.map(TeamId),
            template_data: self.template_data,
            sort_order: self.sort_order,
            inherited_from_id: self.inherited_from_id.map(TemplateId),
            creator_id: self.creator_id.map(UserId),
            last_updated_by_id: self.last_updated_by_id.map(UserId),
            last_applied_at: self.last_applied_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        })
    }
}

#[async_trait]
impl TemplateRepo for PgTx {
    async fn insert(&mut self, t: &NewTemplate) -> AppResult<Template> {
        let row = sqlx::query_as::<_, Row>(&format!(
            "INSERT INTO template (id, organization_id, type, name, description, icon, color, team_id, \
             template_data, sort_order, inherited_from_id, creator_id, last_updated_by_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12, $13, $13) RETURNING {COLS}"
        ))
        .bind(t.id.0)
        .bind(self.org.0)
        .bind(t.template_type.as_str())
        .bind(&t.name)
        .bind(&t.description)
        .bind(&t.icon)
        .bind(&t.color)
        .bind(t.team_id.map(|x| x.0))
        .bind(&t.template_data)
        .bind(t.sort_order)
        .bind(t.inherited_from_id.map(|x| x.0))
        .bind(t.creator_id.map(|x| x.0))
        .bind(t.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.into_template()
    }

    async fn get(&mut self, id: TemplateId) -> AppResult<Option<Template>> {
        let row = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLS} FROM template WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(Row::into_template).transpose()
    }

    async fn update(
        &mut self,
        t: &Template,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Template> {
        let row = sqlx::query_as::<_, Row>(&format!(
            "UPDATE template SET name = $3, description = $4, icon = $5, color = $6, \
             template_data = $7, sort_order = $8, last_updated_by_id = $9, updated_at = $10, \
             version = version + 1 WHERE organization_id = $1 AND id = $2 RETURNING {COLS}"
        ))
        .bind(self.org.0)
        .bind(t.id.0)
        .bind(&t.name)
        .bind(&t.description)
        .bind(&t.icon)
        .bind(&t.color)
        .bind(&t.template_data)
        .bind(t.sort_order)
        .bind(updated_by.map(|u| u.0))
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| lineal_app::AppError::not_found("Could not find referenced Template."))?;
        row.into_template()
    }

    async fn archive(&mut self, id: TemplateId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET default_template_for_members_id = NULLIF(default_template_for_members_id, $2), \
             default_template_for_non_members_id = NULLIF(default_template_for_non_members_id, $2), \
             default_project_template_id = NULLIF(default_project_template_id, $2) \
             WHERE organization_id = $1 AND (default_template_for_members_id = $2 \
               OR default_template_for_non_members_id = $2 OR default_project_template_id = $2)",
        )
        .bind(self.org.0)
        .bind(id.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE template SET archived_at = $3, updated_at = $3, version = version + 1 \
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

    async fn list_active(&mut self) -> AppResult<Vec<Template>> {
        let rows = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLS} FROM template WHERE organization_id = $1 AND archived_at IS NULL \
             ORDER BY sort_order, lower(name), id"
        ))
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(Row::into_template).collect()
    }

    async fn find_by_name(
        &mut self,
        team: Option<TeamId>,
        template_type: TemplateType,
        name: &str,
    ) -> AppResult<Option<Template>> {
        let row = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLS} FROM template WHERE organization_id = $1 AND archived_at IS NULL \
             AND type = $2 AND lower(name) = lower($3) AND team_id IS NOT DISTINCT FROM $4"
        ))
        .bind(self.org.0)
        .bind(template_type.as_str())
        .bind(name)
        .bind(team.map(|t| t.0))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(Row::into_template).transpose()
    }

    async fn find_all_by_name(&mut self, name: &str) -> AppResult<Vec<Template>> {
        let rows = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLS} FROM template WHERE organization_id = $1 AND archived_at IS NULL \
             AND lower(name) = lower($2) ORDER BY sort_order, id"
        ))
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(Row::into_template).collect()
    }

    async fn mark_applied(&mut self, id: TemplateId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE template SET last_applied_at = $3 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn mark_object(
        &mut self,
        template_type: TemplateType,
        object: Uuid,
        template: TemplateId,
    ) -> AppResult<()> {
        let table = match template_type {
            TemplateType::Issue => "issue",
            TemplateType::Project => "project",
            TemplateType::Document => "document",
        };
        sqlx::query(&format!(
            "UPDATE {table} SET last_applied_template_id = $3 WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(object)
        .bind(template.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn team_defaults(&mut self, team: TeamId) -> AppResult<TeamDefaults> {
        let row: Option<(Option<Uuid>, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
            "SELECT default_template_for_members_id, default_template_for_non_members_id, \
             default_project_template_id FROM team WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(
            row.map_or_else(TeamDefaults::default, |(m, n, p)| TeamDefaults {
                members: m.map(TemplateId),
                non_members: n.map(TemplateId),
                project: p.map(TemplateId),
            }),
        )
    }

    async fn set_team_default(
        &mut self,
        team: TeamId,
        slot: DefaultSlot,
        template: Option<TemplateId>,
    ) -> AppResult<()> {
        let col = match slot {
            DefaultSlot::Members => "default_template_for_members_id",
            DefaultSlot::NonMembers => "default_template_for_non_members_id",
            DefaultSlot::Project => "default_project_template_id",
        };
        sqlx::query(&format!(
            "UPDATE team SET {col} = $3 WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(team.0)
        .bind(template.map(|t| t.0))
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
