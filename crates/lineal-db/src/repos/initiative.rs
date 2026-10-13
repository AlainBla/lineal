//! `InitiativeRepo` für `PgTx` (WP07): Initiativen, Hierarchie (DAG), Projektzuordnung, Updates.

use super::project::{escape_like, parse_health, parse_res, sets};
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::ports::repos::InitiativeRepo;
use lineal_app::ports::repos::initiative::{
    Initiative, InitiativeFilter, InitiativeLabelRow, InitiativePatch, InitiativeUpdate,
    InitiativeUpdateFilter, NewInitiative, NewInitiativeUpdate,
};
use lineal_app::ports::repos::project::{ReminderSettings, ReminderTarget};
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::{Health, InitiativeStatus};
use lineal_domain::ids::{InitiativeId, InitiativeUpdateId, OrgId, ProjectId, TeamId, UserId};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct InitiativeRow {
    id: Uuid,
    organization_id: Uuid,
    number: i32,
    name: String,
    slug_id: String,
    description: Option<String>,
    content: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    status: String,
    owner_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    lead_team_id: Option<Uuid>,
    priority: i32,
    priority_sort_order: f64,
    sort_order: f64,
    target_date: Option<NaiveDate>,
    target_date_resolution: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    health: Option<String>,
    health_updated_at: Option<DateTime<Utc>>,
    last_update_id: Option<Uuid>,
    update_reminder_mode: String,
    update_reminder_frequency_in_weeks: Option<i32>,
    update_reminders_day: Option<String>,
    update_reminders_hour: Option<i32>,
    include_sub_initiative_projects: bool,
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const INIT_COLS: &str = "i.id, i.organization_id, i.number, i.name, i.slug_id, i.description, i.content, i.color, \
    i.icon, i.status, i.owner_id, i.creator_id, i.lead_team_id, i.priority, i.priority_sort_order, i.sort_order, \
    i.target_date, i.target_date_resolution, i.started_at, i.completed_at, i.health, i.health_updated_at, \
    i.last_update_id, i.update_reminder_mode, i.update_reminder_frequency_in_weeks, i.update_reminders_day, \
    i.update_reminders_hour, i.include_sub_initiative_projects, i.trashed, i.trashed_at, i.archived_at, \
    i.version, i.created_at, i.updated_at";

impl InitiativeRow {
    fn into_initiative(self) -> AppResult<Initiative> {
        Ok(Initiative {
            id: InitiativeId(self.id),
            org_id: OrgId(self.organization_id),
            number: self.number,
            name: self.name,
            slug_id: self.slug_id,
            description: self.description,
            content: self.content,
            color: self.color,
            icon: self.icon,
            status: self
                .status
                .parse::<InitiativeStatus>()
                .map_err(|_| AppError::internal("invalid initiative status in database"))?,
            owner_id: self.owner_id.map(UserId),
            creator_id: self.creator_id.map(UserId),
            lead_team_id: self.lead_team_id.map(TeamId),
            priority: self.priority,
            priority_sort_order: self.priority_sort_order,
            sort_order: self.sort_order,
            target_date: self.target_date,
            target_date_resolution: parse_res(self.target_date_resolution)?,
            started_at: self.started_at,
            completed_at: self.completed_at,
            health: parse_health(self.health)?,
            health_updated_at: self.health_updated_at,
            last_update_id: self.last_update_id.map(InitiativeUpdateId),
            update_reminder_mode: self.update_reminder_mode,
            update_reminder_frequency_in_weeks: self.update_reminder_frequency_in_weeks,
            update_reminders_day: self.update_reminders_day,
            update_reminders_hour: self.update_reminders_hour,
            include_sub_initiative_projects: self.include_sub_initiative_projects,
            trashed: self.trashed,
            trashed_at: self.trashed_at,
            archived_at: self.archived_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct UpdateRow {
    id: Uuid,
    initiative_id: Uuid,
    user_id: Uuid,
    body: String,
    health: String,
    slug_id: String,
    edited_at: Option<DateTime<Utc>>,
    progress_snapshot: Value,
    diff: Option<Value>,
    diff_markdown: Option<String>,
    is_diff_hidden: bool,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const UPDATE_COLS: &str = "u.id, u.initiative_id, u.user_id, u.body, u.health, u.slug_id, u.edited_at, \
    u.progress_snapshot, u.diff, u.diff_markdown, u.is_diff_hidden, u.archived_at, u.version, u.created_at, u.updated_at";

impl UpdateRow {
    fn into_update(self) -> AppResult<InitiativeUpdate> {
        Ok(InitiativeUpdate {
            id: InitiativeUpdateId(self.id),
            initiative_id: InitiativeId(self.initiative_id),
            user_id: UserId(self.user_id),
            body: self.body,
            health: self
                .health
                .parse::<Health>()
                .map_err(|_| AppError::internal("invalid health in database"))?,
            slug_id: self.slug_id,
            edited_at: self.edited_at,
            snapshot: self.progress_snapshot,
            diff: self.diff,
            diff_markdown: self.diff_markdown,
            is_diff_hidden: self.is_diff_hidden,
            archived_at: self.archived_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct LabelRow {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>,
    is_group: bool,
    retired_at: Option<DateTime<Utc>>,
}

impl From<LabelRow> for InitiativeLabelRow {
    fn from(r: LabelRow) -> Self {
        InitiativeLabelRow {
            id: r.id,
            name: r.name,
            parent_id: r.parent_id,
            is_group: r.is_group,
            retired: r.retired_at.is_some(),
        }
    }
}

/// Sichtbarkeitsbedingung: Initiative ohne Lead-Team oder mit Lead-Team aus der sichtbaren Menge.
fn push_visibility<'a>(qb: &mut QueryBuilder<'a, Postgres>, visible: &[TeamId]) {
    let ids: Vec<Uuid> = visible.iter().map(|t| t.0).collect();
    qb.push(" AND (i.lead_team_id IS NULL OR i.lead_team_id = ANY(");
    qb.push_bind(ids);
    qb.push("))");
}

#[async_trait]
impl InitiativeRepo for PgTx {
    async fn next_number(&mut self) -> AppResult<i32> {
        sqlx::query_scalar(
            "UPDATE organization SET last_initiative_number = last_initiative_number + 1 WHERE id = $1 \
             RETURNING last_initiative_number",
        )
        .bind(self.org.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn lock_hierarchy(&mut self) -> AppResult<()> {
        // Advisory-Sperre statt `FOR UPDATE` auf der organization-Zeile: Letzteres kollidiert mit
        // den KEY-SHARE-Sperren jedes Inserts mit `organization_id` (Deadlock, Workspace-Stau).
        self.advisory_lock_inner("initiative_hierarchy", None).await
    }

    async fn insert(&mut self, i: &NewInitiative) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO initiative (id, organization_id, number, name, description, content, color, icon, status, \
             owner_id, creator_id, lead_team_id, priority, sort_order, priority_sort_order, target_date, \
             target_date_resolution, started_at, completed_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14, $15, $16, $17, $18, $19, $19)",
        )
        .bind(i.id.0)
        .bind(self.org.0)
        .bind(i.number)
        .bind(&i.name)
        .bind(&i.description)
        .bind(&i.content)
        .bind(&i.color)
        .bind(&i.icon)
        .bind(i.status.as_str())
        .bind(i.owner_id.map(|u| u.0))
        .bind(i.creator_id.map(|u| u.0))
        .bind(i.lead_team_id.map(|t| t.0))
        .bind(i.priority)
        .bind(i.sort_order)
        .bind(i.target_date)
        .bind(i.target_date_resolution.map(|r| r.as_str()))
        .bind(i.started_at)
        .bind(i.completed_at)
        .bind(i.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            let unique = e
                .as_database_error()
                .and_then(|d| d.code())
                .is_some_and(|c| c == "23505");
            if unique {
                AppError::conflict("An initiative with this name already exists.")
                    .with_details(serde_json::json!({ "field": "name" }))
            } else {
                db_err(e)
            }
        })?;
        Ok(())
    }

    async fn get(&mut self, id: InitiativeId) -> AppResult<Option<Initiative>> {
        let row = sqlx::query_as::<_, InitiativeRow>(&format!(
            "SELECT {INIT_COLS} FROM initiative i WHERE i.organization_id = $1 AND i.id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(InitiativeRow::into_initiative).transpose()
    }

    async fn update(
        &mut self,
        id: InitiativeId,
        p: &InitiativePatch,
        now: DateTime<Utc>,
    ) -> AppResult<Initiative> {
        let mut s = sets::Sets::new("initiative");
        s.set_some("name", p.name.clone());
        s.set_opt("description", p.description.clone());
        s.set_opt("content", p.content.clone());
        s.set_opt("color", p.color.clone());
        s.set_opt("icon", p.icon.clone());
        s.set_some("status", p.status.map(|x| x.as_str().to_owned()));
        s.set_opt("owner_id", p.owner_id.map(|o| o.map(|u| u.0)));
        s.set_opt("lead_team_id", p.lead_team_id.map(|o| o.map(|t| t.0)));
        s.set_some("priority", p.priority);
        s.set_some("sort_order", p.sort_order);
        s.set_opt("target_date", p.target_date);
        s.set_opt(
            "target_date_resolution",
            p.target_date_resolution
                .map(|o| o.map(|r| r.as_str().to_owned())),
        );
        s.set_opt("started_at", p.started_at);
        s.set_opt("completed_at", p.completed_at);
        s.set_opt("health", p.health.map(|o| o.map(|h| h.as_str().to_owned())));
        s.set_opt("health_updated_at", p.health_updated_at);
        s.set_opt("last_update_id", p.last_update_id.map(|o| o.map(|u| u.0)));
        s.set_some("update_reminder_mode", p.update_reminder_mode.clone());
        s.set_opt(
            "update_reminder_frequency_in_weeks",
            p.update_reminder_frequency_in_weeks,
        );
        s.set_opt("update_reminders_day", p.update_reminders_day.clone());
        s.set_opt("update_reminders_hour", p.update_reminders_hour);
        s.set_some(
            "include_sub_initiative_projects",
            p.include_sub_initiative_projects,
        );
        if let Some(t) = p.trashed {
            s.set("trashed", t);
            s.raw(if t {
                "trashed_at = now()"
            } else {
                "trashed_at = NULL"
            });
        }
        s.set_opt("archived_at", p.archived_at);
        s.raw("version = version + 1");
        s.set("updated_at", now);
        let mut qb = s.qb;
        qb.push(" WHERE organization_id = ");
        qb.push_bind(self.org.0);
        qb.push(" AND id = ");
        qb.push_bind(id.0);
        qb.push(format!(" RETURNING {}", INIT_COLS.replace("i.", "")));
        let row = qb
            .build_query_as::<InitiativeRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|e| {
                let unique = e
                    .as_database_error()
                    .and_then(|d| d.code())
                    .is_some_and(|c| c == "23505");
                if unique {
                    AppError::conflict("An initiative with this name already exists.")
                        .with_details(serde_json::json!({ "field": "name" }))
                } else {
                    db_err(e)
                }
            })?;
        row.map(InitiativeRow::into_initiative)
            .transpose()?
            .ok_or_else(|| AppError::not_found("Could not find referenced Initiative."))
    }

    async fn find_by_number(&mut self, number: i32) -> AppResult<Option<Initiative>> {
        let row = sqlx::query_as::<_, InitiativeRow>(&format!(
            "SELECT {INIT_COLS} FROM initiative i WHERE i.organization_id = $1 AND i.number = $2 AND NOT i.trashed"
        ))
        .bind(self.org.0)
        .bind(number)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(InitiativeRow::into_initiative).transpose()
    }

    async fn find_by_slug_id(&mut self, slug_id: &str) -> AppResult<Option<Initiative>> {
        let row = sqlx::query_as::<_, InitiativeRow>(&format!(
            "SELECT {INIT_COLS} FROM initiative i WHERE i.organization_id = $1 AND i.slug_id = $2 AND NOT i.trashed"
        ))
        .bind(self.org.0)
        .bind(slug_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(InitiativeRow::into_initiative).transpose()
    }

    async fn find_by_name(&mut self, name: &str) -> AppResult<Vec<Initiative>> {
        let rows = sqlx::query_as::<_, InitiativeRow>(&format!(
            "SELECT {INIT_COLS} FROM initiative i WHERE i.organization_id = $1 AND lower(i.name) = lower($2) \
             AND NOT i.trashed AND i.archived_at IS NULL ORDER BY i.created_at, i.id"
        ))
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(InitiativeRow::into_initiative)
            .collect()
    }

    async fn list(
        &mut self,
        f: &InitiativeFilter,
        page: &PageRequest,
    ) -> AppResult<Page<Initiative>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "i.created_at",
            OrderBy::UpdatedAt => "i.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new("");
        if let Some(ev) = &f.eval {
            crate::filter_sql::push_cte_prefix(&mut qb, ev);
            qb.push(" ");
        }
        qb.push(format!(
            "SELECT {INIT_COLS}, {order_col} AS sort_key FROM initiative i WHERE i.organization_id = "
        ));
        qb.push_bind(self.org.0);
        if let Some(since) = f.trashed_since {
            qb.push(" AND i.trashed AND i.trashed_at >= ")
                .push_bind(since);
        } else {
            qb.push(" AND NOT i.trashed");
            if !f.include_archived {
                qb.push(" AND i.archived_at IS NULL");
            }
        }
        if let Some(ast) = &f.ast {
            let ev = f
                .eval
                .as_ref()
                .ok_or_else(|| AppError::internal("filter without evaluation context"))?;
            let cx = crate::filter_sql::Cx::new(ev);
            qb.push(" AND ");
            crate::filter_sql::push_initiative_filter(&mut qb, &cx, "i", ast)?;
        }
        push_visibility(&mut qb, &f.visible_teams);
        if let Some(s) = f.status {
            qb.push(" AND i.status = ");
            qb.push_bind(s.as_str().to_owned());
        }
        if let Some(o) = f.owner {
            qb.push(" AND i.owner_id = ");
            qb.push_bind(o.0);
        }
        if let Some(t) = f.lead_team {
            qb.push(" AND i.lead_team_id = ");
            qb.push_bind(t.0);
        }
        if let Some(p) = f.parent {
            qb.push(
                " AND EXISTS (SELECT 1 FROM initiative_parent ip WHERE ip.child_id = i.id AND ip.parent_id = ",
            );
            qb.push_bind(p.0);
            qb.push(")");
        }
        if let Some(l) = &f.label_ids {
            qb.push(
                " AND EXISTS (SELECT 1 FROM initiative_to_label il WHERE il.initiative_id = i.id AND il.label_id = ANY(",
            );
            qb.push_bind(l.clone());
            qb.push("))");
        }
        if let Some(q) = f.query.as_deref().filter(|q| !q.is_empty()) {
            qb.push(" AND lower(i.name) LIKE ");
            qb.push_bind(format!("%{}%", escape_like(&q.to_lowercase())));
            qb.push(" ESCAPE '\\'");
        }
        if let Some(t) = f.created_after {
            qb.push(" AND i.created_at > ");
            qb.push_bind(t);
        }
        if let Some(t) = f.updated_after {
            qb.push(" AND i.updated_at > ");
            qb.push_bind(t);
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, i.id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, i.id DESC LIMIT "));
        qb.push_bind(page.limit + 1);
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            i: InitiativeRow,
            sort_key: DateTime<Utc>,
        }
        let mut rows = qb
            .build_query_as::<Row>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let has_next = rows.len() as i64 > page.limit;
        if has_next {
            rows.truncate(page.limit as usize);
        }
        let next = if has_next {
            rows.last().map(|r| PageCursor {
                key: r.sort_key,
                id: r.i.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.i.into_initiative())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn list_all(&mut self) -> AppResult<Vec<Initiative>> {
        let rows = sqlx::query_as::<_, InitiativeRow>(&format!(
            "SELECT {INIT_COLS} FROM initiative i WHERE i.organization_id = $1 AND NOT i.trashed \
             ORDER BY i.number"
        ))
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(InitiativeRow::into_initiative)
            .collect()
    }

    async fn list_edges(&mut self) -> AppResult<Vec<(InitiativeId, InitiativeId)>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT parent_id, child_id FROM initiative_parent WHERE organization_id = $1 \
             ORDER BY sort_order, created_at, child_id",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(p, c)| (InitiativeId(p), InitiativeId(c)))
            .collect())
    }

    async fn insert_edge(
        &mut self,
        parent: InitiativeId,
        child: InitiativeId,
        sort_order: f64,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO initiative_parent (organization_id, parent_id, child_id, sort_order, created_at) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (parent_id, child_id) DO NOTHING",
        )
        .bind(self.org.0)
        .bind(parent.0)
        .bind(child.0)
        .bind(sort_order)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_edge(&mut self, parent: InitiativeId, child: InitiativeId) -> AppResult<bool> {
        let res = sqlx::query(
            "DELETE FROM initiative_parent WHERE organization_id = $1 AND parent_id = $2 AND child_id = $3",
        )
        .bind(self.org.0)
        .bind(parent.0)
        .bind(child.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn max_child_sort_order(&mut self, parent: InitiativeId) -> AppResult<f64> {
        let v: Option<f64> = sqlx::query_scalar(
            "SELECT max(sort_order) FROM initiative_parent WHERE organization_id = $1 AND parent_id = $2",
        )
        .bind(self.org.0)
        .bind(parent.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(v.unwrap_or(0.0))
    }

    async fn project_ids(&mut self, id: InitiativeId) -> AppResult<Vec<ProjectId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM initiative_to_project WHERE organization_id = $1 AND initiative_id = $2 \
             ORDER BY sort_order, created_at, project_id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectId).collect())
    }

    async fn initiative_ids_of_project(
        &mut self,
        project: ProjectId,
    ) -> AppResult<Vec<InitiativeId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT initiative_id FROM initiative_to_project WHERE organization_id = $1 AND project_id = $2 \
             ORDER BY created_at, initiative_id",
        )
        .bind(self.org.0)
        .bind(project.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(InitiativeId).collect())
    }

    async fn link_project(
        &mut self,
        initiative: InitiativeId,
        project: ProjectId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO initiative_to_project (organization_id, initiative_id, project_id, sort_order, created_at) \
             VALUES ($1, $2, $3, COALESCE((SELECT max(sort_order) FROM initiative_to_project WHERE initiative_id = $2), 0) + 1, $4) \
             ON CONFLICT (initiative_id, project_id) DO NOTHING",
        )
        .bind(self.org.0)
        .bind(initiative.0)
        .bind(project.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn unlink_project(
        &mut self,
        initiative: InitiativeId,
        project: ProjectId,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "DELETE FROM initiative_to_project WHERE organization_id = $1 AND initiative_id = $2 AND project_id = $3",
        )
        .bind(self.org.0)
        .bind(initiative.0)
        .bind(project.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn initiatives_of_projects(
        &mut self,
        projects: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, InitiativeId)>> {
        let raw: Vec<Uuid> = projects.iter().map(|p| p.0).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT project_id, initiative_id FROM initiative_to_project WHERE organization_id = $1 \
             AND project_id = ANY($2) ORDER BY created_at, initiative_id",
        )
        .bind(self.org.0)
        .bind(raw)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(p, i)| (ProjectId(p), InitiativeId(i)))
            .collect())
    }

    async fn labels_of(&mut self, id: InitiativeId) -> AppResult<Vec<InitiativeLabelRow>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            "SELECT l.id, l.name, l.parent_id, l.is_group, l.retired_at FROM initiative_to_label il \
             JOIN initiative_label l ON l.organization_id = il.organization_id AND l.id = il.label_id \
             WHERE il.organization_id = $1 AND il.initiative_id = $2 ORDER BY lower(l.name), l.id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(InitiativeLabelRow::from).collect())
    }

    async fn get_label(&mut self, id: Uuid) -> AppResult<Option<InitiativeLabelRow>> {
        let row = sqlx::query_as::<_, LabelRow>(
            "SELECT id, name, parent_id, is_group, retired_at FROM initiative_label \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(InitiativeLabelRow::from))
    }

    async fn find_labels_by_name(&mut self, name: &str) -> AppResult<Vec<InitiativeLabelRow>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            "SELECT id, name, parent_id, is_group, retired_at FROM initiative_label \
             WHERE organization_id = $1 AND lower(name) = lower($2) AND archived_at IS NULL ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(InitiativeLabelRow::from).collect())
    }

    async fn set_labels(
        &mut self,
        id: InitiativeId,
        labels: &[InitiativeLabelRow],
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM initiative_to_label WHERE organization_id = $1 AND initiative_id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        for l in labels {
            sqlx::query(
                "INSERT INTO initiative_to_label (organization_id, initiative_id, label_id, group_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(self.org.0)
            .bind(id.0)
            .bind(l.id)
            .bind(l.parent_id)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn insert_update(&mut self, u: &NewInitiativeUpdate) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO initiative_update (id, organization_id, initiative_id, user_id, body, health, \
             progress_snapshot, diff, diff_markdown, is_diff_hidden, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(u.id.0)
        .bind(self.org.0)
        .bind(u.initiative_id.0)
        .bind(u.user_id.0)
        .bind(&u.body)
        .bind(u.health.as_str())
        .bind(&u.snapshot)
        .bind(&u.diff)
        .bind(&u.diff_markdown)
        .bind(u.is_diff_hidden)
        .bind(u.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_update(&mut self, id: InitiativeUpdateId) -> AppResult<Option<InitiativeUpdate>> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "SELECT {UPDATE_COLS} FROM initiative_update u WHERE u.organization_id = $1 AND u.id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UpdateRow::into_update).transpose()
    }

    async fn latest_update(
        &mut self,
        initiative: InitiativeId,
        except: Option<InitiativeUpdateId>,
    ) -> AppResult<Option<InitiativeUpdate>> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "SELECT {UPDATE_COLS} FROM initiative_update u WHERE u.organization_id = $1 AND u.initiative_id = $2 \
             AND u.archived_at IS NULL AND ($3::uuid IS NULL OR u.id <> $3) \
             ORDER BY u.created_at DESC, u.id DESC LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(initiative.0)
        .bind(except.map(|e| e.0))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UpdateRow::into_update).transpose()
    }

    async fn list_updates(
        &mut self,
        f: &InitiativeUpdateFilter,
        page: &PageRequest,
    ) -> AppResult<Page<InitiativeUpdate>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "u.created_at",
            OrderBy::UpdatedAt => "u.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {UPDATE_COLS}, {order_col} AS sort_key FROM initiative_update u \
             JOIN initiative i ON i.organization_id = u.organization_id AND i.id = u.initiative_id \
             WHERE u.organization_id = "
        ));
        qb.push_bind(self.org.0);
        qb.push(" AND NOT i.trashed");
        if !f.include_archived {
            qb.push(" AND u.archived_at IS NULL");
        }
        push_visibility(&mut qb, &f.visible_teams);
        if let Some(i) = f.initiative {
            qb.push(" AND u.initiative_id = ");
            qb.push_bind(i.0);
        }
        if let Some(user) = f.user {
            qb.push(" AND u.user_id = ");
            qb.push_bind(user.0);
        }
        if let Some(t) = f.created_after {
            qb.push(" AND u.created_at > ");
            qb.push_bind(t);
        }
        if let Some(t) = f.updated_after {
            qb.push(" AND u.updated_at > ");
            qb.push_bind(t);
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, u.id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, u.id DESC LIMIT "));
        qb.push_bind(page.limit + 1);
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            update: UpdateRow,
            sort_key: DateTime<Utc>,
        }
        let mut rows = qb
            .build_query_as::<Row>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let has_next = rows.len() as i64 > page.limit;
        if has_next {
            rows.truncate(page.limit as usize);
        }
        let next = if has_next {
            rows.last().map(|r| PageCursor {
                key: r.sort_key,
                id: r.update.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.update.into_update())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn edit_update(
        &mut self,
        id: InitiativeUpdateId,
        body: Option<&str>,
        health: Option<Health>,
        now: DateTime<Utc>,
    ) -> AppResult<InitiativeUpdate> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "UPDATE initiative_update u SET body = COALESCE($3, u.body), health = COALESCE($4, u.health), \
             edited_at = $5, version = u.version + 1, updated_at = $5 \
             WHERE u.organization_id = $1 AND u.id = $2 RETURNING {UPDATE_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(body)
        .bind(health.map(|h| h.as_str()))
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UpdateRow::into_update)
            .transpose()?
            .ok_or_else(|| AppError::not_found("Could not find referenced status update."))
    }

    async fn archive_update(
        &mut self,
        id: InitiativeUpdateId,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE initiative_update SET archived_at = $3, version = version + 1, updated_at = $3 \
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

    async fn update_comment_count(&mut self, id: InitiativeUpdateId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM comment WHERE organization_id = $1 AND initiative_update_id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn reminder_targets(&mut self) -> AppResult<Vec<ReminderTarget>> {
        #[derive(sqlx::FromRow)]
        struct Rw {
            id: Uuid,
            name: String,
            owner_id: Option<Uuid>,
            timezone: Option<String>,
            update_reminder_mode: String,
            update_reminder_frequency_in_weeks: Option<i32>,
            update_reminders_day: Option<String>,
            update_reminders_hour: Option<i32>,
            last_update_at: Option<DateTime<Utc>>,
            since: DateTime<Utc>,
        }
        let rows = sqlx::query_as::<_, Rw>(
            "SELECT i.id, i.name, i.owner_id, u.timezone, i.update_reminder_mode, \
             i.update_reminder_frequency_in_weeks, i.update_reminders_day, i.update_reminders_hour, \
             (SELECT max(iu.created_at) FROM initiative_update iu WHERE iu.initiative_id = i.id AND iu.archived_at IS NULL) \
               AS last_update_at, COALESCE(i.started_at, i.created_at) AS since \
             FROM initiative i LEFT JOIN \"user\" u ON u.organization_id = i.organization_id AND u.id = i.owner_id \
             WHERE i.organization_id = $1 AND i.status = 'Active' AND i.owner_id IS NOT NULL AND NOT i.trashed \
             AND i.archived_at IS NULL ORDER BY i.number",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| ReminderTarget {
                target_id: r.id,
                name: r.name,
                recipient_id: r.owner_id.map(UserId),
                recipient_timezone: r.timezone,
                mode: r.update_reminder_mode,
                frequency_in_weeks: r.update_reminder_frequency_in_weeks,
                day: r.update_reminders_day,
                hour: r.update_reminders_hour,
                paused_until: None,
                last_update_at: r.last_update_at,
                since: r.since,
            })
            .collect())
    }

    async fn reminder_settings(&mut self) -> AppResult<Option<ReminderSettings>> {
        // Gleiche Einstellungen wie im Projekt-Repo (Workspace-Zeile).
        let row: Option<(Uuid, Option<i32>, String, i32, Option<i32>, String, i32)> = sqlx::query_as(
            "SELECT id, project_update_reminder_frequency_in_weeks, project_update_reminders_day, \
             project_update_reminders_hour, initiative_update_reminder_frequency_in_weeks, \
             initiative_update_reminders_day, initiative_update_reminders_hour \
             FROM organization WHERE id = $1",
        )
        .bind(self.org.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(id, pf, pd, ph, i_f, id_, ih)| ReminderSettings {
            org_id: OrgId(id),
            project_frequency_in_weeks: pf,
            project_day: pd,
            project_hour: ph,
            initiative_frequency_in_weeks: i_f,
            initiative_day: id_,
            initiative_hour: ih,
        }))
    }
}
