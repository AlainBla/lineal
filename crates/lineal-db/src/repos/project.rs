//! `ProjectRepo` für `PgTx` (WP07): Projekte, Projektstatus, Meilensteine, Status-Updates.

use super::cycle::ScopeRow;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::ports::repos::ProjectRepo;
use lineal_app::ports::repos::cycle::{HistoryPoint, IssueScopeRow};
use lineal_app::ports::repos::project::{
    ExternalLink, Milestone, MilestonePatch, NewMilestone, NewProject, NewProjectUpdate, Project,
    ProjectFilter, ProjectLabelRow, ProjectPatch, ProjectStatus, ProjectUpdate,
    ProjectUpdateFilter, ReminderSettings, ReminderTarget,
};
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::{DateResolution, Health, ProjectStatusType};
use lineal_domain::ids::{IssueId, MilestoneId, OrgId, ProjectId, ProjectUpdateId, TeamId, UserId};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

/// Hilfen für dynamische `UPDATE … SET`-Listen (auch von `initiative.rs` benutzt).
pub(crate) mod sets {
    use super::*;

    pub(crate) struct Sets<'q> {
        pub qb: QueryBuilder<'q, Postgres>,
        any: bool,
    }

    impl<'q> Sets<'q> {
        pub(crate) fn new(table: &str) -> Self {
            Self {
                qb: QueryBuilder::new(format!("UPDATE {table} SET ")),
                any: false,
            }
        }
        fn sep(&mut self) {
            if self.any {
                self.qb.push(", ");
            }
            self.any = true;
        }
        pub(crate) fn set<T>(&mut self, col: &str, v: T)
        where
            T: 'q + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres> + Send,
        {
            self.sep();
            self.qb.push(format!("{col} = "));
            self.qb.push_bind(v);
        }
        /// `Option<Option<T>>`: `None` = unverändert, `Some(None)` = NULL.
        pub(crate) fn set_opt<T>(&mut self, col: &str, v: Option<Option<T>>)
        where
            T: 'q + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres> + Send,
        {
            if let Some(inner) = v {
                self.sep();
                self.qb.push(format!("{col} = "));
                self.qb.push_bind(inner);
            }
        }
        pub(crate) fn set_some<T>(&mut self, col: &str, v: Option<T>)
        where
            T: 'q + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres> + Send,
        {
            if let Some(inner) = v {
                self.set(col, inner);
            }
        }
        pub(crate) fn raw(&mut self, fragment: &str) {
            self.sep();
            self.qb.push(fragment);
        }
    }
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(crate) fn parse_res(s: Option<String>) -> AppResult<Option<DateResolution>> {
    s.map(|s| {
        s.parse::<DateResolution>()
            .map_err(|_| AppError::internal("invalid date resolution in database"))
    })
    .transpose()
}

pub(crate) fn parse_health(s: Option<String>) -> AppResult<Option<Health>> {
    s.map(|s| {
        s.parse::<Health>()
            .map_err(|_| AppError::internal("invalid health in database"))
    })
    .transpose()
}

#[derive(sqlx::FromRow)]
struct StatusRow {
    id: Uuid,
    name: String,
    #[sqlx(rename = "type")]
    status_type: String,
    color: String,
    description: Option<String>,
    position: f64,
    indefinite: bool,
    team_id: Option<Uuid>,
    inherited_from_id: Option<Uuid>,
    archived_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct ProjectRow {
    id: Uuid,
    organization_id: Uuid,
    name: String,
    lead_team_id: Uuid,
    number: i32,
    slug_id: String,
    description: Option<String>,
    content: Option<String>,
    color: String,
    icon: Option<String>,
    status_id: Uuid,
    priority: i32,
    sort_order: f64,
    priority_sort_order: f64,
    lead_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    start_date: Option<NaiveDate>,
    start_date_resolution: Option<String>,
    target_date: Option<NaiveDate>,
    target_date_resolution: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    canceled_at: Option<DateTime<Utc>>,
    health: Option<String>,
    health_updated_at: Option<DateTime<Utc>>,
    last_update_id: Option<Uuid>,
    update_reminder_mode: String,
    update_reminder_frequency_in_weeks: Option<i32>,
    update_reminders_day: Option<String>,
    update_reminders_hour: Option<i32>,
    update_reminders_paused_until_at: Option<DateTime<Utc>>,
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    auto_archived_at: Option<DateTime<Utc>>,
    scope_history: Vec<f64>,
    completed_scope_history: Vec<f64>,
    in_progress_scope_history: Vec<f64>,
    issue_count_history: Vec<f64>,
    completed_issue_count_history: Vec<f64>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) const PROJECT_COLS: &str = "p.id, p.organization_id, p.name, p.lead_team_id, p.number, p.slug_id, \
    p.description, p.content, p.color, p.icon, p.status_id, p.priority, p.sort_order, p.priority_sort_order, \
    p.lead_id, p.creator_id, p.start_date, p.start_date_resolution, p.target_date, p.target_date_resolution, \
    p.started_at, p.completed_at, p.canceled_at, p.health, p.health_updated_at, p.last_update_id, \
    p.update_reminder_mode, p.update_reminder_frequency_in_weeks, p.update_reminders_day, \
    p.update_reminders_hour, p.update_reminders_paused_until_at, p.trashed, p.trashed_at, p.archived_at, \
    p.auto_archived_at, p.scope_history, p.completed_scope_history, p.in_progress_scope_history, \
    p.issue_count_history, p.completed_issue_count_history, p.version, p.created_at, p.updated_at";

impl ProjectRow {
    fn into_project(self) -> AppResult<Project> {
        Ok(Project {
            id: ProjectId(self.id),
            org_id: OrgId(self.organization_id),
            name: self.name,
            lead_team_id: TeamId(self.lead_team_id),
            number: self.number,
            slug_id: self.slug_id,
            description: self.description,
            content: self.content,
            color: self.color,
            icon: self.icon,
            status_id: self.status_id,
            priority: self.priority,
            sort_order: self.sort_order,
            priority_sort_order: self.priority_sort_order,
            lead_id: self.lead_id.map(UserId),
            creator_id: self.creator_id.map(UserId),
            start_date: self.start_date,
            start_date_resolution: parse_res(self.start_date_resolution)?,
            target_date: self.target_date,
            target_date_resolution: parse_res(self.target_date_resolution)?,
            started_at: self.started_at,
            completed_at: self.completed_at,
            canceled_at: self.canceled_at,
            health: parse_health(self.health)?,
            health_updated_at: self.health_updated_at,
            last_update_id: self.last_update_id.map(ProjectUpdateId),
            update_reminder_mode: self.update_reminder_mode,
            update_reminder_frequency_in_weeks: self.update_reminder_frequency_in_weeks,
            update_reminders_day: self.update_reminders_day,
            update_reminders_hour: self.update_reminders_hour,
            update_reminders_paused_until_at: self.update_reminders_paused_until_at,
            trashed: self.trashed,
            trashed_at: self.trashed_at,
            archived_at: self.archived_at,
            auto_archived_at: self.auto_archived_at,
            scope_history: self.scope_history,
            completed_scope_history: self.completed_scope_history,
            in_progress_scope_history: self.in_progress_scope_history,
            issue_count_history: self.issue_count_history,
            completed_issue_count_history: self.completed_issue_count_history,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct MilestoneRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    description: Option<String>,
    target_date: Option<NaiveDate>,
    sort_order: f64,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const MILESTONE_COLS: &str = "id, project_id, name, description, target_date, sort_order, archived_at, version, created_at, updated_at";

impl From<MilestoneRow> for Milestone {
    fn from(r: MilestoneRow) -> Self {
        Milestone {
            id: MilestoneId(r.id),
            project_id: ProjectId(r.project_id),
            name: r.name,
            description: r.description,
            target_date: r.target_date,
            sort_order: r.sort_order,
            archived_at: r.archived_at,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct UpdateRow {
    id: Uuid,
    project_id: Uuid,
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

const UPDATE_COLS: &str = "u.id, u.project_id, u.user_id, u.body, u.health, u.slug_id, u.edited_at, \
    u.progress_snapshot, u.diff, u.diff_markdown, u.is_diff_hidden, u.archived_at, u.version, u.created_at, u.updated_at";

impl UpdateRow {
    fn into_update(self) -> AppResult<ProjectUpdate> {
        Ok(ProjectUpdate {
            id: ProjectUpdateId(self.id),
            project_id: ProjectId(self.project_id),
            user_id: UserId(self.user_id),
            body: self.body,
            health: self
                .health
                .parse()
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
    team_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    is_group: bool,
    retired_at: Option<DateTime<Utc>>,
}

impl From<LabelRow> for ProjectLabelRow {
    fn from(r: LabelRow) -> Self {
        ProjectLabelRow {
            id: r.id,
            name: r.name,
            team_id: r.team_id.map(TeamId),
            parent_id: r.parent_id,
            is_group: r.is_group,
            retired: r.retired_at.is_some(),
        }
    }
}

pub(crate) fn pad_set(arr: &mut Vec<f64>, idx: usize, value: f64) {
    let last = arr.last().copied().unwrap_or(0.0);
    while arr.len() < idx {
        arr.push(last);
    }
    if arr.len() == idx {
        arr.push(value);
    } else {
        arr[idx] = value;
    }
}

#[async_trait]
impl ProjectRepo for PgTx {
    async fn list_statuses(&mut self) -> AppResult<Vec<ProjectStatus>> {
        let rows = sqlx::query_as::<_, StatusRow>(
            "SELECT id, name, type, color, description, position, indefinite, team_id, inherited_from_id, archived_at \
             FROM project_status WHERE organization_id = $1 \
             ORDER BY CASE type WHEN 'backlog' THEN 0 WHEN 'planned' THEN 1 WHEN 'started' THEN 2 \
               WHEN 'paused' THEN 3 WHEN 'completed' THEN 4 ELSE 5 END, position, created_at, id",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|r| {
                Ok(ProjectStatus {
                    id: r.id,
                    name: r.name,
                    status_type: r
                        .status_type
                        .parse::<ProjectStatusType>()
                        .map_err(|_| AppError::internal("invalid project status type"))?,
                    color: r.color,
                    description: r.description,
                    position: r.position,
                    indefinite: r.indefinite,
                    team_id: r.team_id.map(TeamId),
                    inherited_from_id: r.inherited_from_id,
                    archived_at: r.archived_at,
                })
            })
            .collect()
    }

    async fn insert(&mut self, p: &NewProject) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project (id, organization_id, name, lead_team_id, number, description, content, color, icon, \
             status_id, priority, sort_order, priority_sort_order, lead_id, creator_id, start_date, \
             start_date_resolution, target_date, target_date_resolution, started_at, completed_at, canceled_at, \
             created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, COALESCE($8, '#5E6AD2'), $9, $10, $11, $12, $12, $13, $14, $15, \
             $16, $17, $18, $19, $20, $21, $22, $22)",
        )
        .bind(p.id.0)
        .bind(self.org.0)
        .bind(&p.name)
        .bind(p.lead_team_id.0)
        .bind(p.number)
        .bind(&p.description)
        .bind(&p.content)
        .bind(&p.color)
        .bind(&p.icon)
        .bind(p.status_id)
        .bind(p.priority)
        .bind(p.sort_order)
        .bind(p.lead_id.map(|u| u.0))
        .bind(p.creator_id.map(|u| u.0))
        .bind(p.start_date)
        .bind(p.start_date_resolution.map(|r| r.as_str()))
        .bind(p.target_date)
        .bind(p.target_date_resolution.map(|r| r.as_str()))
        .bind(p.started_at)
        .bind(p.completed_at)
        .bind(p.canceled_at)
        .bind(p.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get(&mut self, id: ProjectId) -> AppResult<Option<Project>> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {PROJECT_COLS} FROM project p WHERE p.organization_id = $1 AND p.id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(ProjectRow::into_project).transpose()
    }

    async fn update(
        &mut self,
        id: ProjectId,
        patch: &ProjectPatch,
        now: DateTime<Utc>,
    ) -> AppResult<Project> {
        let mut s = sets::Sets::new("project");
        s.set_some("name", patch.name.clone());
        s.set_some("lead_team_id", patch.lead_team_id.map(|t| t.0));
        s.set_some("number", patch.number);
        s.set_opt("description", patch.description.clone());
        s.set_opt("content", patch.content.clone());
        s.set_some("color", patch.color.clone());
        s.set_opt("icon", patch.icon.clone());
        s.set_some("status_id", patch.status_id);
        s.set_some("priority", patch.priority);
        s.set_some("sort_order", patch.sort_order);
        s.set_opt("lead_id", patch.lead_id.map(|o| o.map(|u| u.0)));
        s.set_opt("start_date", patch.start_date);
        s.set_opt(
            "start_date_resolution",
            patch
                .start_date_resolution
                .map(|o| o.map(|r| r.as_str().to_owned())),
        );
        s.set_opt("target_date", patch.target_date);
        s.set_opt(
            "target_date_resolution",
            patch
                .target_date_resolution
                .map(|o| o.map(|r| r.as_str().to_owned())),
        );
        s.set_opt("started_at", patch.started_at);
        s.set_opt("completed_at", patch.completed_at);
        s.set_opt("canceled_at", patch.canceled_at);
        s.set_opt(
            "health",
            patch.health.map(|o| o.map(|h| h.as_str().to_owned())),
        );
        s.set_opt("health_updated_at", patch.health_updated_at);
        s.set_opt(
            "last_update_id",
            patch.last_update_id.map(|o| o.map(|u| u.0)),
        );
        s.set_some("update_reminder_mode", patch.update_reminder_mode.clone());
        s.set_opt(
            "update_reminder_frequency_in_weeks",
            patch.update_reminder_frequency_in_weeks,
        );
        s.set_opt("update_reminders_day", patch.update_reminders_day.clone());
        s.set_opt("update_reminders_hour", patch.update_reminders_hour);
        s.set_opt(
            "update_reminders_paused_until_at",
            patch.update_reminders_paused_until_at,
        );
        if let Some(t) = patch.trashed {
            s.set("trashed", t);
            s.raw(if t {
                "trashed_at = now()"
            } else {
                "trashed_at = NULL"
            });
        }
        s.set_opt("archived_at", patch.archived_at);
        s.raw("version = version + 1");
        s.set("updated_at", now);
        let mut qb = s.qb;
        qb.push(" WHERE organization_id = ");
        qb.push_bind(self.org.0);
        qb.push(" AND id = ");
        qb.push_bind(id.0);
        qb.push(format!(" RETURNING {}", PROJECT_COLS.replace("p.", "")));
        let row = qb
            .build_query_as::<ProjectRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        row.map(ProjectRow::into_project)
            .transpose()?
            .ok_or_else(|| AppError::not_found("Could not find referenced Project."))
    }

    async fn find_by_key_and_number(
        &mut self,
        team_key: &str,
        number: i32,
    ) -> AppResult<Option<Project>> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {PROJECT_COLS} FROM project p JOIN team t ON t.organization_id = p.organization_id \
             AND t.id = p.lead_team_id WHERE p.organization_id = $1 AND upper(t.key) = upper($2) \
             AND p.number = $3 AND NOT t.trashed AND NOT p.trashed"
        ))
        .bind(self.org.0)
        .bind(team_key)
        .bind(number)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(ProjectRow::into_project).transpose()
    }

    async fn find_by_alias(&mut self, identifier: &str) -> AppResult<Option<Project>> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {PROJECT_COLS} FROM project p JOIN issue_identifier_alias a ON a.organization_id = p.organization_id \
             AND a.project_id = p.id WHERE p.organization_id = $1 AND a.entity_type = 'project' \
             AND a.identifier = upper($2) AND NOT p.trashed"
        ))
        .bind(self.org.0)
        .bind(identifier)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(ProjectRow::into_project).transpose()
    }

    async fn find_by_slug_id(&mut self, slug_id: &str) -> AppResult<Option<Project>> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {PROJECT_COLS} FROM project p WHERE p.organization_id = $1 AND p.slug_id = $2 AND NOT p.trashed"
        ))
        .bind(self.org.0)
        .bind(slug_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(ProjectRow::into_project).transpose()
    }

    async fn find_by_name(&mut self, name: &str) -> AppResult<Vec<Project>> {
        let rows = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {PROJECT_COLS} FROM project p WHERE p.organization_id = $1 AND lower(p.name) = lower($2) \
             AND NOT p.trashed AND p.archived_at IS NULL ORDER BY p.created_at, p.id"
        ))
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ProjectRow::into_project).collect()
    }

    async fn list(&mut self, f: &ProjectFilter, page: &PageRequest) -> AppResult<Page<Project>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "p.created_at",
            OrderBy::UpdatedAt => "p.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new("");
        if let Some(ev) = &f.eval {
            crate::filter_sql::push_cte_prefix(&mut qb, ev);
            qb.push(" ");
        }
        qb.push(format!(
            "SELECT {PROJECT_COLS}, {order_col} AS sort_key FROM project p WHERE p.organization_id = "
        ));
        qb.push_bind(self.org.0);
        qb.push(" AND NOT p.trashed");
        if !f.include_archived {
            qb.push(" AND p.archived_at IS NULL");
        }
        let visible: Vec<Uuid> = f.visible_teams.iter().map(|t| t.0).collect();
        qb.push(
            " AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY(",
        );
        qb.push_bind(visible);
        qb.push("))");
        if let Some(ast) = &f.ast {
            let ev = f
                .eval
                .as_ref()
                .ok_or_else(|| AppError::internal("filter without evaluation context"))?;
            let cx = crate::filter_sql::Cx::new(ev);
            qb.push(" AND ");
            crate::filter_sql::push_project_filter(&mut qb, &cx, "p", ast)?;
        }
        if let Some(teams) = &f.teams {
            let ids: Vec<Uuid> = teams.iter().map(|t| t.0).collect();
            qb.push(
                " AND EXISTS (SELECT 1 FROM project_team pt2 WHERE pt2.project_id = p.id AND pt2.team_id = ANY(",
            );
            qb.push_bind(ids);
            qb.push("))");
        }
        if let Some(inits) = &f.initiatives {
            let ids: Vec<Uuid> = inits.iter().map(|t| t.0).collect();
            qb.push(
                " AND EXISTS (SELECT 1 FROM initiative_to_project ip WHERE ip.project_id = p.id AND ip.initiative_id = ANY(",
            );
            qb.push_bind(ids);
            qb.push("))");
        }
        if let Some(u) = f.member {
            qb.push(" AND (p.lead_id = ");
            qb.push_bind(u.0);
            qb.push(" OR EXISTS (SELECT 1 FROM project_member pm WHERE pm.project_id = p.id AND pm.user_id = ");
            qb.push_bind(u.0);
            qb.push("))");
        }
        if let Some(st) = &f.status_ids {
            qb.push(" AND p.status_id = ANY(");
            qb.push_bind(st.clone());
            qb.push(")");
        }
        if let Some(l) = &f.label_ids {
            qb.push(
                " AND EXISTS (SELECT 1 FROM project_to_label pl WHERE pl.project_id = p.id AND pl.label_id = ANY(",
            );
            qb.push_bind(l.clone());
            qb.push("))");
        }
        if let Some(q) = f.query.as_deref().filter(|q| !q.is_empty()) {
            let pattern = format!("%{}%", escape_like(&q.to_lowercase()));
            qb.push(" AND (lower(p.name) LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" ESCAPE '\\' OR lower(coalesce(p.description, '')) LIKE ");
            qb.push_bind(pattern);
            qb.push(" ESCAPE '\\')");
        }
        if let Some(t) = f.created_after {
            qb.push(" AND p.created_at > ");
            qb.push_bind(t);
        }
        if let Some(t) = f.updated_after {
            qb.push(" AND p.updated_at > ");
            qb.push_bind(t);
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, p.id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, p.id DESC LIMIT "));
        qb.push_bind(page.limit + 1);

        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            project: ProjectRow,
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
                id: r.project.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.project.into_project())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn previous_identifiers(&mut self, id: ProjectId) -> AppResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT identifier FROM issue_identifier_alias WHERE organization_id = $1 AND project_id = $2 \
             AND entity_type = 'project' ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn add_alias(
        &mut self,
        id: ProjectId,
        identifier: &str,
        team_key: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_identifier_alias (organization_id, identifier, entity_type, project_id, team_key) \
             VALUES ($1, upper($2), 'project', $3, $4) ON CONFLICT (organization_id, identifier) DO NOTHING",
        )
        .bind(self.org.0)
        .bind(identifier)
        .bind(id.0)
        .bind(team_key)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn ids_for_team(&mut self, team: TeamId) -> AppResult<Vec<ProjectId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM project_team WHERE organization_id = $1 AND team_id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectId).collect())
    }

    async fn list_teams(&mut self, id: ProjectId) -> AppResult<Vec<TeamId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT team_id FROM project_team WHERE organization_id = $1 AND project_id = $2 \
             ORDER BY sort_order, created_at, team_id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(TeamId).collect())
    }

    async fn add_team(
        &mut self,
        id: ProjectId,
        team: TeamId,
        sort_order: f64,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project_team (organization_id, project_id, team_id, sort_order, created_at) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (project_id, team_id) DO NOTHING",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(team.0)
        .bind(sort_order)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn remove_team(&mut self, id: ProjectId, team: TeamId) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM project_team WHERE organization_id = $1 AND project_id = $2 AND team_id = $3",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(team.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn teams_of_projects(
        &mut self,
        ids: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, TeamId)>> {
        let raw: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT project_id, team_id FROM project_team WHERE organization_id = $1 AND project_id = ANY($2) \
             ORDER BY sort_order, created_at, team_id",
        )
        .bind(self.org.0)
        .bind(raw)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(p, t)| (ProjectId(p), TeamId(t)))
            .collect())
    }

    async fn list_members(&mut self, id: ProjectId) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM project_member WHERE organization_id = $1 AND project_id = $2 \
             ORDER BY created_at, user_id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn add_member(
        &mut self,
        id: ProjectId,
        user: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project_member (organization_id, project_id, user_id, created_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (project_id, user_id) DO NOTHING",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(user.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn remove_member(&mut self, id: ProjectId, user: UserId) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM project_member WHERE organization_id = $1 AND project_id = $2 AND user_id = $3",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(user.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn labels_of(&mut self, id: ProjectId) -> AppResult<Vec<ProjectLabelRow>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            "SELECT l.id, l.name, l.team_id, l.parent_id, l.is_group, l.retired_at FROM project_to_label pl \
             JOIN project_label l ON l.organization_id = pl.organization_id AND l.id = pl.label_id \
             WHERE pl.organization_id = $1 AND pl.project_id = $2 ORDER BY lower(l.name), l.id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectLabelRow::from).collect())
    }

    async fn get_label(&mut self, id: Uuid) -> AppResult<Option<ProjectLabelRow>> {
        let row = sqlx::query_as::<_, LabelRow>(
            "SELECT id, name, team_id, parent_id, is_group, retired_at FROM project_label \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(ProjectLabelRow::from))
    }

    async fn find_labels_by_name(&mut self, name: &str) -> AppResult<Vec<ProjectLabelRow>> {
        let rows = sqlx::query_as::<_, LabelRow>(
            "SELECT id, name, team_id, parent_id, is_group, retired_at FROM project_label \
             WHERE organization_id = $1 AND lower(name) = lower($2) AND archived_at IS NULL ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectLabelRow::from).collect())
    }

    async fn set_labels(
        &mut self,
        id: ProjectId,
        labels: &[ProjectLabelRow],
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query("DELETE FROM project_to_label WHERE organization_id = $1 AND project_id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        for l in labels {
            sqlx::query(
                "INSERT INTO project_to_label (organization_id, project_id, label_id, group_id, created_at) \
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

    async fn list_links(&mut self, id: ProjectId) -> AppResult<Vec<ExternalLink>> {
        let rows: Vec<(Uuid, String, String, f64)> = sqlx::query_as(
            "SELECT id, url, label, sort_order FROM entity_external_link WHERE organization_id = $1 \
             AND project_id = $2 AND archived_at IS NULL ORDER BY sort_order, created_at, id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, url, label, sort_order)| ExternalLink {
                id,
                url,
                label,
                sort_order,
            })
            .collect())
    }

    async fn add_link(
        &mut self,
        id: ProjectId,
        url: &str,
        label: &str,
        creator: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO entity_external_link (organization_id, url, label, project_id, sort_order, creator_id, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, \
             COALESCE((SELECT max(sort_order) FROM entity_external_link WHERE project_id = $4), 0) + 1, $5, $6, $6) \
             ON CONFLICT (project_id, url) WHERE project_id IS NOT NULL DO NOTHING",
        )
        .bind(self.org.0)
        .bind(url)
        .bind(label)
        .bind(id.0)
        .bind(creator.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn document_count(&mut self, id: ProjectId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM document WHERE organization_id = $1 AND project_id = $2 \
             AND NOT trashed AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn list_documents(&mut self, id: ProjectId) -> AppResult<Vec<(Uuid, String, String)>> {
        sqlx::query_as(
            "SELECT id, title, slug_id FROM document WHERE organization_id = $1 AND project_id = $2 \
             AND NOT trashed AND archived_at IS NULL ORDER BY sort_order, created_at, id",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn subscribe(
        &mut self,
        id: ProjectId,
        user: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO notification_subscription (organization_id, subscriber_id, target_type, project_id, \
             created_at, updated_at) VALUES ($1, $2, 'project', $3, $4, $4) ON CONFLICT DO NOTHING",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn issue_rows(&mut self, id: ProjectId) -> AppResult<Vec<IssueScopeRow>> {
        let rows = sqlx::query_as::<_, ScopeRow>(
            "SELECT i.id, i.team_id, i.state_type, i.estimate, t.issue_estimation_type, t.default_issue_estimate \
             FROM issue i JOIN team t ON t.organization_id = i.organization_id AND t.id = i.team_id \
             WHERE i.organization_id = $1 AND i.project_id = $2 AND NOT i.trashed",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ScopeRow::into_row).collect()
    }

    async fn issue_rows_of_projects(
        &mut self,
        ids: &[ProjectId],
    ) -> AppResult<Vec<(ProjectId, IssueScopeRow)>> {
        #[derive(sqlx::FromRow)]
        struct Rw {
            project_id: Uuid,
            #[sqlx(flatten)]
            row: ScopeRow,
        }
        let raw: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        let rows = sqlx::query_as::<_, Rw>(
            "SELECT i.project_id, i.id, i.team_id, i.state_type, i.estimate, t.issue_estimation_type, \
             t.default_issue_estimate FROM issue i JOIN team t ON t.organization_id = i.organization_id \
             AND t.id = i.team_id WHERE i.organization_id = $1 AND i.project_id = ANY($2) AND NOT i.trashed",
        )
        .bind(self.org.0)
        .bind(raw)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|r| Ok((ProjectId(r.project_id), r.row.into_row()?)))
            .collect()
    }

    async fn open_issue_count(&mut self, id: ProjectId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM issue WHERE organization_id = $1 AND project_id = $2 AND NOT trashed \
             AND state_type IN ('triage','backlog','unstarted','started')",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn issues_of_team(
        &mut self,
        id: ProjectId,
        team: TeamId,
        limit: i64,
    ) -> AppResult<Vec<(IssueId, String)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, identifier FROM issue WHERE organization_id = $1 AND project_id = $2 AND team_id = $3 \
             AND NOT trashed ORDER BY number LIMIT $4",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(team.0)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(i, s)| (IssueId(i), s)).collect())
    }

    async fn detach_team_issues(
        &mut self,
        id: ProjectId,
        team: TeamId,
        actor: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<IssueId>> {
        let rows: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
            "SELECT id, project_milestone_id FROM issue WHERE organization_id = $1 AND project_id = $2 \
             AND team_id = $3 ORDER BY number FOR UPDATE",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for (issue, milestone) in rows {
            sqlx::query(
                "UPDATE issue SET project_id = NULL, project_milestone_id = NULL, version = version + 1, \
                 updated_at = $3 WHERE organization_id = $1 AND id = $2",
            )
            .bind(self.org.0)
            .bind(issue)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            let mut changes = serde_json::Map::new();
            changes.insert("projectId".into(), json!({ "from": id.0, "to": null }));
            if let Some(m) = milestone {
                changes.insert(
                    "projectMilestoneId".into(),
                    json!({ "from": m, "to": null }),
                );
            }
            sqlx::query(
                "INSERT INTO issue_history (organization_id, issue_id, actor_id, source, changes, created_at) \
                 VALUES ($1, $2, $3, 'user', $4, $5)",
            )
            .bind(self.org.0)
            .bind(issue)
            .bind(actor.map(|a| a.0))
            .bind(Value::Object(changes))
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            out.push(IssueId(issue));
        }
        Ok(out)
    }

    async fn list_milestones(&mut self, id: ProjectId) -> AppResult<Vec<Milestone>> {
        let rows = sqlx::query_as::<_, MilestoneRow>(&format!(
            "SELECT {MILESTONE_COLS} FROM project_milestone WHERE organization_id = $1 AND project_id = $2 \
             AND archived_at IS NULL ORDER BY sort_order, created_at, id"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Milestone::from).collect())
    }

    async fn get_milestone(&mut self, id: MilestoneId) -> AppResult<Option<Milestone>> {
        let row = sqlx::query_as::<_, MilestoneRow>(&format!(
            "SELECT {MILESTONE_COLS} FROM project_milestone WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Milestone::from))
    }

    async fn find_milestone_by_name(
        &mut self,
        project: ProjectId,
        name: &str,
    ) -> AppResult<Option<Milestone>> {
        let row = sqlx::query_as::<_, MilestoneRow>(&format!(
            "SELECT {MILESTONE_COLS} FROM project_milestone WHERE organization_id = $1 AND project_id = $2 \
             AND lower(name) = lower($3) AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(project.0)
        .bind(name)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Milestone::from))
    }

    async fn insert_milestone(&mut self, m: &NewMilestone) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project_milestone (id, organization_id, project_id, name, description, target_date, \
             sort_order, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(m.id.0)
        .bind(self.org.0)
        .bind(m.project_id.0)
        .bind(&m.name)
        .bind(&m.description)
        .bind(m.target_date)
        .bind(m.sort_order)
        .bind(m.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            let unique = e
                .as_database_error()
                .and_then(|d| d.code())
                .is_some_and(|c| c == "23505");
            if unique {
                AppError::invalid_request("A milestone with this name already exists in the project.")
                    .with_details(json!({ "field": "name" }))
            } else {
                db_err(e)
            }
        })?;
        Ok(())
    }

    async fn update_milestone(
        &mut self,
        id: MilestoneId,
        patch: &MilestonePatch,
        now: DateTime<Utc>,
    ) -> AppResult<Milestone> {
        let mut s = sets::Sets::new("project_milestone");
        s.set_some("name", patch.name.clone());
        s.set_opt("description", patch.description.clone());
        s.set_opt("target_date", patch.target_date);
        s.set_some("sort_order", patch.sort_order);
        s.raw("version = version + 1");
        s.set("updated_at", now);
        let mut qb = s.qb;
        qb.push(" WHERE organization_id = ");
        qb.push_bind(self.org.0);
        qb.push(" AND id = ");
        qb.push_bind(id.0);
        qb.push(format!(" RETURNING {MILESTONE_COLS}"));
        let row = qb
            .build_query_as::<MilestoneRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|e| {
                let unique = e
                    .as_database_error()
                    .and_then(|d| d.code())
                    .is_some_and(|c| c == "23505");
                if unique {
                    AppError::invalid_request(
                        "A milestone with this name already exists in the project.",
                    )
                    .with_details(json!({ "field": "name" }))
                } else {
                    db_err(e)
                }
            })?;
        row.map(Milestone::from)
            .ok_or_else(|| AppError::not_found("Could not find referenced Milestone."))
    }

    async fn max_milestone_sort_order(&mut self, project: ProjectId) -> AppResult<f64> {
        // Parallele Anlage: Das Maximum erst unter Sperre lesen (gilt bis Commit), sonst vergeben
        // zwei Aufrufe dieselbe Position (max + 1).
        self.advisory_lock_inner("milestone_sort", Some(project.0))
            .await?;
        let v: Option<f64> = sqlx::query_scalar(
            "SELECT max(sort_order) FROM project_milestone WHERE organization_id = $1 AND project_id = $2",
        )
        .bind(self.org.0)
        .bind(project.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(v.unwrap_or(0.0))
    }

    async fn archive_milestone(
        &mut self,
        id: MilestoneId,
        actor: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<(IssueId, TeamId)>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT id, team_id FROM issue WHERE organization_id = $1 AND project_milestone_id = $2 ORDER BY number FOR UPDATE",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for (issue, issue_team) in rows {
            sqlx::query(
                "UPDATE issue SET project_milestone_id = NULL, version = version + 1, updated_at = $3 \
                 WHERE organization_id = $1 AND id = $2",
            )
            .bind(self.org.0)
            .bind(issue)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            sqlx::query(
                "INSERT INTO issue_history (organization_id, issue_id, actor_id, source, changes, created_at) \
                 VALUES ($1, $2, $3, 'user', $4, $5)",
            )
            .bind(self.org.0)
            .bind(issue)
            .bind(actor.map(|a| a.0))
            .bind(json!({ "projectMilestoneId": { "from": id.0, "to": null } }))
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            out.push((IssueId(issue), TeamId(issue_team)));
        }
        sqlx::query(
            "UPDATE project_milestone SET archived_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(out)
    }

    async fn milestone_issue_rows(
        &mut self,
        project: ProjectId,
    ) -> AppResult<Vec<(MilestoneId, IssueScopeRow)>> {
        #[derive(sqlx::FromRow)]
        struct Rw {
            project_milestone_id: Uuid,
            #[sqlx(flatten)]
            row: ScopeRow,
        }
        let rows = sqlx::query_as::<_, Rw>(
            "SELECT i.project_milestone_id, i.id, i.team_id, i.state_type, i.estimate, t.issue_estimation_type, \
             t.default_issue_estimate FROM issue i JOIN team t ON t.organization_id = i.organization_id \
             AND t.id = i.team_id WHERE i.organization_id = $1 AND i.project_id = $2 \
             AND i.project_milestone_id IS NOT NULL AND NOT i.trashed",
        )
        .bind(self.org.0)
        .bind(project.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|r| Ok((MilestoneId(r.project_milestone_id), r.row.into_row()?)))
            .collect()
    }

    async fn insert_update(&mut self, u: &NewProjectUpdate) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO project_update (id, organization_id, project_id, user_id, body, health, progress_snapshot, \
             diff, diff_markdown, is_diff_hidden, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(u.id.0)
        .bind(self.org.0)
        .bind(u.project_id.0)
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

    async fn get_update(&mut self, id: ProjectUpdateId) -> AppResult<Option<ProjectUpdate>> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "SELECT {UPDATE_COLS} FROM project_update u WHERE u.organization_id = $1 AND u.id = $2"
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
        project: ProjectId,
        except: Option<ProjectUpdateId>,
    ) -> AppResult<Option<ProjectUpdate>> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "SELECT {UPDATE_COLS} FROM project_update u WHERE u.organization_id = $1 AND u.project_id = $2 \
             AND u.archived_at IS NULL AND ($3::uuid IS NULL OR u.id <> $3) \
             ORDER BY u.created_at DESC, u.id DESC LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(project.0)
        .bind(except.map(|e| e.0))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UpdateRow::into_update).transpose()
    }

    async fn list_updates(
        &mut self,
        f: &ProjectUpdateFilter,
        page: &PageRequest,
    ) -> AppResult<Page<ProjectUpdate>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "u.created_at",
            OrderBy::UpdatedAt => "u.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {UPDATE_COLS}, {order_col} AS sort_key FROM project_update u \
             JOIN project p ON p.organization_id = u.organization_id AND p.id = u.project_id \
             WHERE u.organization_id = "
        ));
        qb.push_bind(self.org.0);
        qb.push(" AND NOT p.trashed");
        if !f.include_archived {
            qb.push(" AND u.archived_at IS NULL");
        }
        let visible: Vec<Uuid> = f.visible_teams.iter().map(|t| t.0).collect();
        qb.push(
            " AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY(",
        );
        qb.push_bind(visible);
        qb.push("))");
        if let Some(p) = f.project {
            qb.push(" AND u.project_id = ");
            qb.push_bind(p.0);
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
        id: ProjectUpdateId,
        body: Option<&str>,
        health: Option<Health>,
        now: DateTime<Utc>,
    ) -> AppResult<ProjectUpdate> {
        let row = sqlx::query_as::<_, UpdateRow>(&format!(
            "UPDATE project_update u SET body = COALESCE($3, u.body), health = COALESCE($4, u.health), \
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

    async fn archive_update(&mut self, id: ProjectUpdateId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE project_update SET archived_at = $3, version = version + 1, updated_at = $3 \
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

    async fn update_comment_count(&mut self, id: ProjectUpdateId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM comment WHERE organization_id = $1 AND project_update_id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn reminder_settings(&mut self) -> AppResult<Option<ReminderSettings>> {
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

    async fn reminder_targets(&mut self) -> AppResult<Vec<ReminderTarget>> {
        #[derive(sqlx::FromRow)]
        struct Rw {
            id: Uuid,
            name: String,
            lead_id: Option<Uuid>,
            timezone: Option<String>,
            update_reminder_mode: String,
            update_reminder_frequency_in_weeks: Option<i32>,
            update_reminders_day: Option<String>,
            update_reminders_hour: Option<i32>,
            update_reminders_paused_until_at: Option<DateTime<Utc>>,
            last_update_at: Option<DateTime<Utc>>,
            since: DateTime<Utc>,
        }
        let rows = sqlx::query_as::<_, Rw>(
            "SELECT p.id, p.name, p.lead_id, u.timezone, p.update_reminder_mode, \
             p.update_reminder_frequency_in_weeks, p.update_reminders_day, p.update_reminders_hour, \
             p.update_reminders_paused_until_at, \
             (SELECT max(pu.created_at) FROM project_update pu WHERE pu.project_id = p.id AND pu.archived_at IS NULL) \
               AS last_update_at, COALESCE(p.started_at, p.created_at) AS since \
             FROM project p JOIN project_status s ON s.organization_id = p.organization_id AND s.id = p.status_id \
             LEFT JOIN \"user\" u ON u.organization_id = p.organization_id AND u.id = p.lead_id \
             WHERE p.organization_id = $1 AND s.type = 'started' AND NOT s.indefinite AND NOT p.trashed \
             AND p.archived_at IS NULL ORDER BY p.created_at, p.id",
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
                recipient_id: r.lead_id.map(UserId),
                recipient_timezone: r.timezone,
                mode: r.update_reminder_mode,
                frequency_in_weeks: r.update_reminder_frequency_in_weeks,
                day: r.update_reminders_day,
                hour: r.update_reminders_hour,
                paused_until: r.update_reminders_paused_until_at,
                last_update_at: r.last_update_at,
                since: r.since,
            })
            .collect())
    }

    async fn orgs_with_planning(&mut self) -> AppResult<Vec<OrgId>> {
        let rows: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM organization WHERE archived_at IS NULL ORDER BY id")
                .fetch_all(&mut *self.tx)
                .await
                .map_err(db_err)?;
        Ok(rows.into_iter().map(OrgId).collect())
    }

    async fn reminder_sent(
        &mut self,
        target_type: &str,
        target: Uuid,
        due_at: DateTime<Utc>,
        stage: i32,
    ) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM update_reminder_sent WHERE organization_id = $1 AND target_type = $2 \
             AND target_id = $3 AND due_at = $4 AND stage = $5)",
        )
        .bind(self.org.0)
        .bind(target_type)
        .bind(target)
        .bind(due_at)
        .bind(stage)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn record_reminder(
        &mut self,
        target_type: &str,
        target: Uuid,
        recipient: UserId,
        due_at: DateTime<Utc>,
        stage: i32,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let res = sqlx::query(
            "INSERT INTO update_reminder_sent (organization_id, target_type, target_id, due_at, stage, sent_at) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
        )
        .bind(self.org.0)
        .bind(target_type)
        .bind(target)
        .bind(due_at)
        .bind(stage)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Ok(());
        }
        let (ntype, project, initiative) = if target_type == "project" {
            ("projectUpdatePrompt", Some(target), None)
        } else {
            ("initiativeUpdatePrompt", None, Some(target))
        };
        sqlx::query(
            "INSERT INTO notification (organization_id, user_id, type, category, target_type, project_id, \
             initiative_id, created_at, updated_at) VALUES ($1, $2, $3, 'reminders', $4, $5, $6, $7, $7)",
        )
        .bind(self.org.0)
        .bind(recipient.0)
        .bind(ntype)
        .bind(target_type)
        .bind(project)
        .bind(initiative)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn write_history_point(
        &mut self,
        id: ProjectId,
        idx: usize,
        point: HistoryPoint,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let Some(mut p) = ProjectRepo::get(self, id).await? else {
            return Ok(());
        };
        pad_set(&mut p.scope_history, idx, point.scope);
        pad_set(&mut p.completed_scope_history, idx, point.completed_scope);
        pad_set(
            &mut p.in_progress_scope_history,
            idx,
            point.in_progress_scope,
        );
        pad_set(&mut p.issue_count_history, idx, point.issue_count);
        pad_set(
            &mut p.completed_issue_count_history,
            idx,
            point.completed_issue_count,
        );
        sqlx::query(
            "UPDATE project SET scope_history = $3, completed_scope_history = $4, in_progress_scope_history = $5, \
             issue_count_history = $6, completed_issue_count_history = $7 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(&p.scope_history)
        .bind(&p.completed_scope_history)
        .bind(&p.in_progress_scope_history)
        .bind(&p.issue_count_history)
        .bind(&p.completed_issue_count_history)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let _ = now;
        Ok(())
    }

    async fn active_project_ids(&mut self) -> AppResult<Vec<ProjectId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT p.id FROM project p JOIN project_status s ON s.organization_id = p.organization_id \
             AND s.id = p.status_id WHERE p.organization_id = $1 AND NOT p.trashed AND p.archived_at IS NULL \
             AND s.type NOT IN ('completed','canceled') ORDER BY p.created_at, p.id",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectId).collect())
    }
}
