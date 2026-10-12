//! `ViewRepo` für `PgTx` (WP11): Custom Views, Anzeigeoptionen, Favoriten, Referenzprüfung.
//!
//! Jede Abfrage trägt `organization_id`; persönliche Daten (Views, Preferences, Favoriten)
//! zusätzlich den User. Die Sichtbarkeit einer View ([`push_view_visible`]) steht in jeder
//! Abfrage, die View-Zeilen liefert – unsichtbare Views existieren für den Aufrufer nicht.

use crate::db_err;
use crate::filter_sql::{
    FilterRefs, RefKind, collect_initiative_refs, collect_issue_refs, collect_project_refs,
};
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::ViewRepo;
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use lineal_app::ports::repos::view::{
    AnyFilter, CustomView, Favorite, FilterEval, NewCustomView, NewFavorite, PrefLevel, ViewAccess,
    ViewKey, ViewListFilter, ViewModel, ViewSubscription,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::{InitiativeId, ProjectId, TeamId, UserId, ViewId};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

const VIEW_COLS: &str = "v.id, v.name, v.description, v.icon, v.color, v.slug_id, v.model_name, \
    v.filter_data, v.project_filter_data, v.initiative_filter_data, v.owner_id, v.creator_id, \
    v.updated_by_id, v.shared, v.team_id, v.project_id, v.initiative_id, v.sort_order, v.version, \
    v.created_at, v.updated_at, v.archived_at";

#[derive(sqlx::FromRow)]
struct ViewRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    slug_id: String,
    model_name: String,
    filter_data: Option<Value>,
    project_filter_data: Option<Value>,
    initiative_filter_data: Option<Value>,
    owner_id: Uuid,
    creator_id: Uuid,
    updated_by_id: Option<Uuid>,
    shared: bool,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
    initiative_id: Option<Uuid>,
    sort_order: f64,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
}

impl ViewRow {
    fn into_view(self) -> AppResult<CustomView> {
        let model = ViewModel::parse(&self.model_name)
            .ok_or_else(|| AppError::internal("unknown view model"))?;
        let filter = match model {
            ViewModel::Issue => self.filter_data,
            ViewModel::Project => self.project_filter_data,
            ViewModel::Initiative => self.initiative_filter_data,
        }
        .unwrap_or(Value::Null);
        Ok(CustomView {
            id: ViewId(self.id),
            name: self.name,
            description: self.description,
            icon: self.icon,
            color: self.color,
            slug_id: self.slug_id,
            model,
            filter,
            owner_id: UserId(self.owner_id),
            creator_id: UserId(self.creator_id),
            updated_by_id: self.updated_by_id.map(UserId),
            shared: self.shared,
            team_id: self.team_id.map(TeamId),
            project_id: self.project_id.map(ProjectId),
            initiative_id: self.initiative_id.map(InitiativeId),
            sort_order: self.sort_order,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        })
    }
}

fn team_ids(v: &[TeamId]) -> Vec<Uuid> {
    v.iter().map(|t| t.0).collect()
}

/// Sichtbarkeitsprädikat für `custom_view v` (docs/05 §4.2), beginnend mit dem Mandanten.
/// Persönliche Views sieht nur der Owner, Team-Views Mitglieder (bei Gästen enthält
/// `visible_teams` nur ihre Teams), Workspace-Views alle Nicht-Gäste; Projekt-/Initiativ-Tabs
/// sieht, wer das Projekt bzw. die Initiative sieht.
fn push_view_visible<'q>(qb: &mut QueryBuilder<'q, Postgres>, org: Uuid, a: &ViewAccess) {
    let vis = team_ids(&a.visible_teams);
    qb.push(" v.organization_id = ");
    qb.push_bind(org);
    qb.push(" AND ((v.project_id IS NULL AND v.initiative_id IS NULL AND v.owner_id = ");
    qb.push_bind(a.user_id.0);
    qb.push(" AND (v.team_id IS NULL OR v.team_id = ANY(");
    qb.push_bind(vis.clone());
    qb.push(")))");
    qb.push(
        " OR (v.shared AND v.project_id IS NULL AND v.initiative_id IS NULL AND v.team_id IS NULL AND NOT ",
    );
    qb.push_bind(a.is_guest);
    qb.push(")");
    qb.push(" OR (v.shared AND v.team_id = ANY(");
    qb.push_bind(vis.clone());
    qb.push("))");
    qb.push(" OR (v.project_id IS NOT NULL AND (v.shared OR v.owner_id = ");
    qb.push_bind(a.user_id.0);
    qb.push(
        ") AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = v.project_id AND pt.team_id = ANY(",
    );
    qb.push_bind(vis.clone());
    qb.push(")))");
    qb.push(" OR (v.initiative_id IS NOT NULL AND NOT ");
    qb.push_bind(a.is_guest);
    qb.push(" AND (v.shared OR v.owner_id = ");
    qb.push_bind(a.user_id.0);
    qb.push(
        ") AND EXISTS (SELECT 1 FROM initiative n WHERE n.id = v.initiative_id AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY(",
    );
    qb.push_bind(vis);
    qb.push("))))) ");
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn favorite_target_column(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "issue" => "issue_id",
        "project" => "project_id",
        "cycle" => "cycle_id",
        "customView" => "custom_view_id",
        "document" => "document_id",
        "initiative" => "initiative_id",
        "label" => "label_id",
        "projectLabel" => "project_label_id",
        "team" => "team_id",
        "user" => "user_id",
        _ => return None,
    })
}

const FAV_COLS: &str = "id, owner_id, type, parent_id, folder_name, sort_order, issue_id, project_id, \
    cycle_id, custom_view_id, document_id, initiative_id, label_id, project_label_id, team_id, \
    user_id, predefined_view_type, predefined_view_team_id, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct FavRow {
    id: Uuid,
    owner_id: Uuid,
    r#type: String,
    parent_id: Option<Uuid>,
    folder_name: Option<String>,
    sort_order: f64,
    issue_id: Option<Uuid>,
    project_id: Option<Uuid>,
    cycle_id: Option<Uuid>,
    custom_view_id: Option<Uuid>,
    document_id: Option<Uuid>,
    initiative_id: Option<Uuid>,
    label_id: Option<Uuid>,
    project_label_id: Option<Uuid>,
    team_id: Option<Uuid>,
    user_id: Option<Uuid>,
    predefined_view_type: Option<String>,
    predefined_view_team_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl FavRow {
    fn into_favorite(self) -> Favorite {
        let target_id = [
            self.issue_id,
            self.project_id,
            self.cycle_id,
            self.custom_view_id,
            self.document_id,
            self.initiative_id,
            self.label_id,
            self.project_label_id,
            self.team_id,
            self.user_id,
        ]
        .into_iter()
        .flatten()
        .next();
        Favorite {
            id: self.id,
            owner_id: UserId(self.owner_id),
            kind: self.r#type,
            parent_id: self.parent_id,
            folder_name: self.folder_name,
            sort_order: self.sort_order,
            target_id,
            predefined_view_type: self.predefined_view_type,
            predefined_view_team_id: self.predefined_view_team_id.map(TeamId),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn not_found_ref(kind: &str, id: Uuid) -> AppError {
    AppError::not_found(format!("Could not find referenced {kind}."))
        .with_details(json!({ "reference": id.to_string(), "type": kind }))
}

#[async_trait]
impl ViewRepo for PgTx {
    async fn search(
        &mut self,
        q: &lineal_app::ports::repos::view::SearchQuery,
    ) -> AppResult<Vec<lineal_app::ports::repos::view::SearchHitRow>> {
        crate::search::search_all(&mut self.tx, self.org.0, q).await
    }

    async fn insert_view(&mut self, v: &NewCustomView) -> AppResult<CustomView> {
        let (fd, pfd, ifd) = match v.model {
            ViewModel::Issue => (Some(&v.filter), None, None),
            ViewModel::Project => (None, Some(&v.filter), None),
            ViewModel::Initiative => (None, None, Some(&v.filter)),
        };
        let sql = format!(
            "INSERT INTO custom_view AS v (id, organization_id, name, description, icon, color, \
             model_name, filter_data, project_filter_data, initiative_filter_data, owner_id, \
             creator_id, shared, team_id, project_id, initiative_id, sort_order, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$18) \
             RETURNING {VIEW_COLS}"
        );
        sqlx::query_as::<_, ViewRow>(&sql)
            .bind(v.id.0)
            .bind(self.org.0)
            .bind(&v.name)
            .bind(&v.description)
            .bind(&v.icon)
            .bind(&v.color)
            .bind(v.model.as_str())
            .bind(fd)
            .bind(pfd)
            .bind(ifd)
            .bind(v.owner_id.0)
            .bind(v.creator_id.0)
            .bind(v.shared)
            .bind(v.team_id.map(|t| t.0))
            .bind(v.project_id.map(|p| p.0))
            .bind(v.initiative_id.map(|i| i.0))
            .bind(v.sort_order)
            .bind(v.now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)?
            .into_view()
    }

    async fn get_view(&mut self, access: &ViewAccess, id: ViewId) -> AppResult<Option<CustomView>> {
        let mut qb: QueryBuilder<'_, Postgres> =
            QueryBuilder::new(format!("SELECT {VIEW_COLS} FROM custom_view v WHERE"));
        push_view_visible(&mut qb, self.org.0, access);
        qb.push(" AND v.id = ");
        qb.push_bind(id.0);
        qb.build_query_as::<ViewRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?
            .map(ViewRow::into_view)
            .transpose()
    }

    async fn find_views(
        &mut self,
        access: &ViewAccess,
        reference: &str,
    ) -> AppResult<Vec<CustomView>> {
        let r = reference.trim();
        // Stufe 1: UUID, Stufe 2: slugId, Stufe 3: exakter Name.
        let stages: [Option<(&str, String)>; 3] = [
            Uuid::parse_str(r).ok().map(|u| ("v.id = ", u.to_string())),
            (r.len() == 12 && r.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| ("v.slug_id = ", r.to_lowercase())),
            Some(("lower(v.name) = lower(", r.to_owned())),
        ];
        for stage in stages.into_iter().flatten() {
            let mut qb: QueryBuilder<'_, Postgres> =
                QueryBuilder::new(format!("SELECT {VIEW_COLS} FROM custom_view v WHERE"));
            push_view_visible(&mut qb, self.org.0, access);
            qb.push(" AND v.archived_at IS NULL AND ");
            qb.push(stage.0);
            if stage.0.starts_with("v.id") {
                qb.push_bind(Uuid::parse_str(&stage.1).unwrap_or_default());
            } else if stage.0.starts_with("lower") {
                qb.push_bind(stage.1);
                qb.push(")");
            } else {
                qb.push_bind(stage.1);
            }
            qb.push(" ORDER BY v.created_at, v.id LIMIT 11");
            let rows = qb
                .build_query_as::<ViewRow>()
                .fetch_all(&mut *self.tx)
                .await
                .map_err(db_err)?;
            if !rows.is_empty() {
                return rows.into_iter().map(ViewRow::into_view).collect();
            }
        }
        Ok(Vec::new())
    }

    async fn list_views(
        &mut self,
        access: &ViewAccess,
        f: &ViewListFilter,
        page: &PageRequest,
    ) -> AppResult<Page<CustomView>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "v.created_at",
            OrderBy::UpdatedAt => "v.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {VIEW_COLS}, {order_col} AS sort_key FROM custom_view v WHERE"
        ));
        push_view_visible(&mut qb, self.org.0, access);
        qb.push(" AND v.archived_at IS NULL");
        if !f.include_scoped {
            qb.push(" AND v.project_id IS NULL AND v.initiative_id IS NULL");
        }
        if let Some(m) = f.model {
            qb.push(" AND v.model_name = ").push_bind(m.as_str());
        }
        if let Some(t) = f.team {
            qb.push(" AND v.team_id = ").push_bind(t.0);
        }
        if let Some(s) = f.shared {
            qb.push(" AND v.shared = ").push_bind(s);
        }
        if let Some(q) = f.query.as_deref().filter(|q| !q.trim().is_empty()) {
            qb.push(" AND lower(v.name) LIKE ");
            qb.push_bind(format!("%{}%", escape_like(&q.trim().to_lowercase())));
            qb.push(" ESCAPE '\\'");
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, v.id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, v.id DESC LIMIT "));
        qb.push_bind(page.limit + 1);
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            view: ViewRow,
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
                id: r.view.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.view.into_view())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn save_view(
        &mut self,
        v: &CustomView,
        updated_by: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<CustomView> {
        let (fd, pfd, ifd) = match v.model {
            ViewModel::Issue => (Some(&v.filter), None, None),
            ViewModel::Project => (None, Some(&v.filter), None),
            ViewModel::Initiative => (None, None, Some(&v.filter)),
        };
        let sql = format!(
            "UPDATE custom_view v SET name = $3, description = $4, icon = $5, color = $6, \
             filter_data = $7, project_filter_data = $8, initiative_filter_data = $9, owner_id = $10, \
             shared = $11, team_id = $12, sort_order = $13, archived_at = $14, updated_by_id = $15, \
             version = v.version + 1, updated_at = $16 \
             WHERE v.organization_id = $1 AND v.id = $2 RETURNING {VIEW_COLS}"
        );
        sqlx::query_as::<_, ViewRow>(&sql)
            .bind(self.org.0)
            .bind(v.id.0)
            .bind(&v.name)
            .bind(&v.description)
            .bind(&v.icon)
            .bind(&v.color)
            .bind(fd)
            .bind(pfd)
            .bind(ifd)
            .bind(v.owner_id.0)
            .bind(v.shared)
            .bind(v.team_id.map(|t| t.0))
            .bind(v.sort_order)
            .bind(v.archived_at)
            .bind(updated_by.0)
            .bind(now)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?
            .ok_or_else(|| AppError::not_found("Could not find referenced CustomView."))?
            .into_view()
    }

    async fn delete_view(&mut self, id: ViewId) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM custom_view WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn verify_filter_refs(&mut self, filter: &AnyFilter, ev: &FilterEval) -> AppResult<()> {
        let refs: FilterRefs = match filter {
            AnyFilter::Issue(f) => collect_issue_refs(ev, f)?,
            AnyFilter::Project(f) => collect_project_refs(ev, f)?,
            AnyFilter::Initiative(f) => collect_initiative_refs(ev, f)?,
        };
        let vis = team_ids(&ev.visible_teams);
        let org = self.org.0;
        for (kind, ids) in refs.0 {
            let ids: Vec<Uuid> = ids.into_iter().collect();
            let (label, sql): (&str, &str) = match kind {
                RefKind::Team => {
                    // Sichtbarkeit steht im Kontext; private Teams ohne Mitgliedschaft fehlen dort.
                    for id in &ids {
                        if !vis.contains(id) {
                            return Err(not_found_ref("Team", *id));
                        }
                    }
                    continue;
                }
                RefKind::User => (
                    "User",
                    "SELECT id FROM \"user\" WHERE organization_id = $1 AND id = ANY($2) AND $3::uuid[] IS NOT NULL",
                ),
                RefKind::State => (
                    "WorkflowState",
                    "SELECT id FROM workflow_state WHERE organization_id = $1 AND id = ANY($2) \
                     AND team_id = ANY($3)",
                ),
                RefKind::Label => (
                    "IssueLabel",
                    "SELECT id FROM issue_label WHERE organization_id = $1 AND id = ANY($2) \
                     AND (team_id IS NULL OR team_id = ANY($3))",
                ),
                RefKind::Cycle => (
                    "Cycle",
                    "SELECT id FROM cycle WHERE organization_id = $1 AND id = ANY($2) AND team_id = ANY($3)",
                ),
                RefKind::Project => (
                    "Project",
                    "SELECT p.id FROM project p WHERE p.organization_id = $1 AND p.id = ANY($2) \
                     AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY($3))",
                ),
                RefKind::Milestone => (
                    "ProjectMilestone",
                    "SELECT m.id FROM project_milestone m WHERE m.organization_id = $1 AND m.id = ANY($2) \
                     AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = m.project_id AND pt.team_id = ANY($3))",
                ),
                RefKind::Template => (
                    "Template",
                    "SELECT id FROM template WHERE organization_id = $1 AND id = ANY($2) \
                     AND (team_id IS NULL OR team_id = ANY($3))",
                ),
                RefKind::ProjectStatus => (
                    "ProjectStatus",
                    "SELECT id FROM project_status WHERE organization_id = $1 AND id = ANY($2) \
                     AND (team_id IS NULL OR team_id = ANY($3))",
                ),
                RefKind::ProjectLabel => (
                    "ProjectLabel",
                    "SELECT id FROM project_label WHERE organization_id = $1 AND id = ANY($2) \
                     AND (team_id IS NULL OR team_id = ANY($3))",
                ),
                RefKind::Initiative => (
                    "Initiative",
                    "SELECT id FROM initiative WHERE organization_id = $1 AND id = ANY($2) \
                     AND (lead_team_id IS NULL OR lead_team_id = ANY($3))",
                ),
                RefKind::InitiativeLabel => (
                    "InitiativeLabel",
                    "SELECT id FROM initiative_label WHERE organization_id = $1 AND id = ANY($2) AND $3::uuid[] IS NOT NULL",
                ),
            };
            let found: Vec<Uuid> = sqlx::query_scalar(sql)
                .bind(org)
                .bind(&ids)
                .bind(&vis)
                .fetch_all(&mut *self.tx)
                .await
                .map_err(db_err)?;
            for id in ids {
                if !found.contains(&id) {
                    return Err(not_found_ref(label, id));
                }
            }
        }
        Ok(())
    }

    async fn get_preferences(
        &mut self,
        key: &ViewKey,
        user: UserId,
    ) -> AppResult<(Option<Value>, Option<Value>)> {
        let rows: Vec<(String, Value)> = sqlx::query_as(
            "SELECT type, preferences FROM view_preferences WHERE organization_id = $1 \
             AND custom_view_id IS NOT DISTINCT FROM $2 AND view_type IS NOT DISTINCT FROM $3 \
             AND view_target_id IS NOT DISTINCT FROM $4 AND archived_at IS NULL \
             AND (type = 'organization' OR user_id = $5)",
        )
        .bind(self.org.0)
        .bind(key.custom_view_id.map(|v| v.0))
        .bind(&key.view_type)
        .bind(key.view_target_id)
        .bind(user.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut org = None;
        let mut mine = None;
        for (t, p) in rows {
            if t == "organization" {
                org = Some(p);
            } else {
                mine = Some(p);
            }
        }
        Ok((org, mine))
    }

    async fn put_preferences(
        &mut self,
        key: &ViewKey,
        level: PrefLevel,
        user: Option<UserId>,
        prefs: &Value,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let user = match level {
            PrefLevel::User => user.map(|u| u.0),
            PrefLevel::Organization => None,
        };
        let updated = sqlx::query(
            "UPDATE view_preferences SET preferences = $7, version = version + 1, updated_at = $8 \
             WHERE organization_id = $1 AND type = $2 AND user_id IS NOT DISTINCT FROM $3 \
             AND custom_view_id IS NOT DISTINCT FROM $4 AND view_type IS NOT DISTINCT FROM $5 \
             AND view_target_id IS NOT DISTINCT FROM $6",
        )
        .bind(self.org.0)
        .bind(level.as_str())
        .bind(user)
        .bind(key.custom_view_id.map(|v| v.0))
        .bind(&key.view_type)
        .bind(key.view_target_id)
        .bind(prefs)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if updated.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO view_preferences (organization_id, type, user_id, custom_view_id, \
                 view_type, view_target_id, preferences, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8)",
            )
            .bind(self.org.0)
            .bind(level.as_str())
            .bind(user)
            .bind(key.custom_view_id.map(|v| v.0))
            .bind(&key.view_type)
            .bind(key.view_target_id)
            .bind(prefs)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn delete_preferences(
        &mut self,
        key: &ViewKey,
        level: PrefLevel,
        user: Option<UserId>,
    ) -> AppResult<bool> {
        let user = match level {
            PrefLevel::User => user.map(|u| u.0),
            PrefLevel::Organization => None,
        };
        let res = sqlx::query(
            "DELETE FROM view_preferences WHERE organization_id = $1 AND type = $2 \
             AND user_id IS NOT DISTINCT FROM $3 AND custom_view_id IS NOT DISTINCT FROM $4 \
             AND view_type IS NOT DISTINCT FROM $5 AND view_target_id IS NOT DISTINCT FROM $6",
        )
        .bind(self.org.0)
        .bind(level.as_str())
        .bind(user)
        .bind(key.custom_view_id.map(|v| v.0))
        .bind(&key.view_type)
        .bind(key.view_target_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_favorites(&mut self, owner: UserId) -> AppResult<Vec<Favorite>> {
        let sql = format!(
            "SELECT {FAV_COLS} FROM favorite WHERE organization_id = $1 AND owner_id = $2 \
             AND archived_at IS NULL ORDER BY sort_order, created_at, id"
        );
        let rows = sqlx::query_as::<_, FavRow>(&sql)
            .bind(self.org.0)
            .bind(owner.0)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(FavRow::into_favorite).collect())
    }

    async fn get_favorite(&mut self, owner: UserId, id: Uuid) -> AppResult<Option<Favorite>> {
        let sql = format!(
            "SELECT {FAV_COLS} FROM favorite WHERE organization_id = $1 AND owner_id = $2 AND id = $3"
        );
        Ok(sqlx::query_as::<_, FavRow>(&sql)
            .bind(self.org.0)
            .bind(owner.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?
            .map(FavRow::into_favorite))
    }

    async fn insert_favorite(&mut self, f: &NewFavorite) -> AppResult<Favorite> {
        let col = favorite_target_column(&f.kind);
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(
            "INSERT INTO favorite (id, organization_id, owner_id, type, parent_id, folder_name, \
             sort_order, predefined_view_type, predefined_view_team_id, created_at, updated_at",
        );
        if let Some(c) = col {
            qb.push(", ").push(c);
        }
        qb.push(") VALUES (");
        qb.push_bind(f.id);
        qb.push(", ").push_bind(self.org.0);
        qb.push(", ").push_bind(f.owner_id.0);
        qb.push(", ").push_bind(f.kind.clone());
        qb.push(", ").push_bind(f.parent_id);
        qb.push(", ").push_bind(f.folder_name.clone());
        qb.push(", ").push_bind(f.sort_order);
        qb.push(", ").push_bind(f.predefined_view_type.clone());
        qb.push(", ")
            .push_bind(f.predefined_view_team_id.map(|t| t.0));
        qb.push(", ").push_bind(f.now);
        qb.push(", ").push_bind(f.now);
        if col.is_some() {
            qb.push(", ").push_bind(f.target_id);
        }
        qb.push(format!(") RETURNING {FAV_COLS}"));
        qb.build_query_as::<FavRow>()
            .fetch_one(&mut *self.tx)
            .await
            .map(FavRow::into_favorite)
            .map_err(|e| crate::repos::org::conflict_or_internal(e, "favorite already exists"))
    }

    async fn delete_favorite(&mut self, owner: UserId, id: Uuid) -> AppResult<bool> {
        // Kinder eines Ordners wandern auf die oberste Ebene (docs/05 §4.6).
        sqlx::query(
            "UPDATE favorite SET parent_id = NULL WHERE organization_id = $1 AND owner_id = $2 AND parent_id = $3",
        )
        .bind(self.org.0)
        .bind(owner.0)
        .bind(id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let res = sqlx::query(
            "DELETE FROM favorite WHERE organization_id = $1 AND owner_id = $2 AND id = $3",
        )
        .bind(self.org.0)
        .bind(owner.0)
        .bind(id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn update_favorite(
        &mut self,
        owner: UserId,
        id: Uuid,
        parent: Option<Uuid>,
        sort_order: f64,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Favorite>> {
        let sql = format!(
            "UPDATE favorite SET parent_id = $4, sort_order = $5, updated_at = $6, version = version + 1 \
             WHERE organization_id = $1 AND owner_id = $2 AND id = $3 RETURNING {FAV_COLS}"
        );
        Ok(sqlx::query_as::<_, FavRow>(&sql)
            .bind(self.org.0)
            .bind(owner.0)
            .bind(id)
            .bind(parent)
            .bind(sort_order)
            .bind(now)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?
            .map(FavRow::into_favorite))
    }

    async fn upsert_view_subscription(
        &mut self,
        user: UserId,
        view: ViewId,
        types: &[String],
        active: bool,
        now: DateTime<Utc>,
    ) -> AppResult<ViewSubscription> {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM notification_subscription WHERE organization_id = $1 AND subscriber_id = $2 \
             AND target_type = 'customView' AND custom_view_id = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(view.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let id = match existing {
            Some(id) => {
                sqlx::query(
                    "UPDATE notification_subscription SET notification_subscription_types = $2, active = $3, \
                     updated_at = $4, version = version + 1 WHERE id = $1",
                )
                .bind(id)
                .bind(types)
                .bind(active)
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                id
            }
            None => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO notification_subscription (id, organization_id, subscriber_id, target_type, \
                     custom_view_id, notification_subscription_types, active, created_at, updated_at) \
                     VALUES ($1,$2,$3,'customView',$4,$5,$6,$7,$7)",
                )
                .bind(id)
                .bind(self.org.0)
                .bind(user.0)
                .bind(view.0)
                .bind(types)
                .bind(active)
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                id
            }
        };
        Ok(ViewSubscription {
            id,
            subscriber_id: user,
            view_id: view,
            types: types.to_vec(),
            active,
        })
    }

    async fn get_view_subscription(
        &mut self,
        user: UserId,
        view: ViewId,
    ) -> AppResult<Option<ViewSubscription>> {
        let row: Option<(Uuid, Vec<String>, bool)> = sqlx::query_as(
            "SELECT id, notification_subscription_types, active FROM notification_subscription \
             WHERE organization_id = $1 AND subscriber_id = $2 AND target_type = 'customView' \
             AND custom_view_id = $3 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(view.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(id, types, active)| ViewSubscription {
            id,
            subscriber_id: user,
            view_id: view,
            types,
            active,
        }))
    }

    async fn active_view_subscriptions(
        &mut self,
        view: ViewId,
        ntype: &str,
    ) -> AppResult<Vec<ViewSubscription>> {
        let rows: Vec<(Uuid, Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT id, subscriber_id, notification_subscription_types FROM notification_subscription \
             WHERE organization_id = $1 AND target_type = 'customView' AND custom_view_id = $2 \
             AND active AND archived_at IS NULL AND $3 = ANY(notification_subscription_types) \
             ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(view.0)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, user, types)| ViewSubscription {
                id,
                subscriber_id: UserId(user),
                view_id: view,
                types,
                active: true,
            })
            .collect())
    }

    async fn subscribed_views(&mut self, ntype: &str) -> AppResult<Vec<ViewSubscription>> {
        let rows: Vec<(Uuid, Uuid, Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT id, custom_view_id, subscriber_id, notification_subscription_types \
             FROM notification_subscription \
             WHERE organization_id = $1 AND target_type = 'customView' AND active \
             AND archived_at IS NULL AND $2 = ANY(notification_subscription_types) \
             ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(ntype)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, view, user, types)| ViewSubscription {
                id,
                subscriber_id: UserId(user),
                view_id: ViewId(view),
                types,
                active: true,
            })
            .collect())
    }

    async fn deactivate_view_subscriptions_except(
        &mut self,
        view: ViewId,
        keep: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE notification_subscription SET active = false, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND target_type = 'customView' AND custom_view_id = $2 \
             AND subscriber_id <> $3 AND active",
        )
        .bind(self.org.0)
        .bind(view.0)
        .bind(keep.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn max_favorite_sort_order(
        &mut self,
        owner: UserId,
        parent: Option<Uuid>,
    ) -> AppResult<Option<f64>> {
        sqlx::query_scalar(
            "SELECT max(sort_order) FROM favorite WHERE organization_id = $1 AND owner_id = $2 \
             AND parent_id IS NOT DISTINCT FROM $3",
        )
        .bind(self.org.0)
        .bind(owner.0)
        .bind(parent)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn favorite_target_visible(
        &mut self,
        kind: &str,
        target: Uuid,
        access: &ViewAccess,
    ) -> AppResult<bool> {
        let vis = team_ids(&access.visible_teams);
        let org = self.org.0;
        let sql = match kind {
            "issue" => {
                "SELECT EXISTS (SELECT 1 FROM issue i WHERE i.organization_id = $1 AND i.id = $2 \
                 AND NOT i.trashed AND (i.team_id = ANY($3) OR EXISTS (SELECT 1 FROM issue_share s \
                 WHERE s.issue_id = i.id AND s.user_id = $4)))"
            }
            "project" => {
                "SELECT EXISTS (SELECT 1 FROM project p WHERE p.organization_id = $1 AND p.id = $2 \
                 AND NOT p.trashed AND EXISTS (SELECT 1 FROM project_team pt \
                 WHERE pt.project_id = p.id AND pt.team_id = ANY($3)))"
            }
            "cycle" => {
                "SELECT EXISTS (SELECT 1 FROM cycle c WHERE c.organization_id = $1 AND c.id = $2 \
                 AND c.team_id = ANY($3))"
            }
            "team" => {
                "SELECT EXISTS (SELECT 1 FROM team t WHERE t.organization_id = $1 AND t.id = $2 \
                 AND NOT t.trashed AND t.id = ANY($3))"
            }
            "user" => {
                "SELECT EXISTS (SELECT 1 FROM \"user\" u WHERE u.organization_id = $1 AND u.id = $2 \
                )"
            }
            "label" => {
                "SELECT EXISTS (SELECT 1 FROM issue_label l WHERE l.organization_id = $1 AND l.id = $2 \
                 AND (l.team_id IS NULL OR l.team_id = ANY($3)))"
            }
            "projectLabel" => {
                "SELECT EXISTS (SELECT 1 FROM project_label l WHERE l.organization_id = $1 AND l.id = $2 \
                 AND (l.team_id IS NULL OR l.team_id = ANY($3)))"
            }
            "initiative" => {
                "SELECT EXISTS (SELECT 1 FROM initiative n WHERE n.organization_id = $1 AND n.id = $2 \
                 AND NOT n.trashed AND NOT $5 AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY($3)))"
            }
            "document" => {
                "SELECT EXISTS (SELECT 1 FROM document d WHERE d.organization_id = $1 AND d.id = $2 \
                 AND NOT d.trashed AND (d.team_id = ANY($3) \
                 OR EXISTS (SELECT 1 FROM issue i WHERE i.id = d.issue_id AND i.team_id = ANY($3)) \
                 OR EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = d.project_id AND pt.team_id = ANY($3)) \
                 OR (NOT $5 AND EXISTS (SELECT 1 FROM initiative n WHERE n.id = d.initiative_id \
                     AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY($3))))))"
            }
            // Eigene Views prüft der Use-Case über `get_view`.
            _ => return Ok(false),
        };
        // Alle fünf Parameter müssen im Statement vorkommen (Extended Protocol).
        let sql = format!(
            "SELECT $3::uuid[] IS NOT NULL AND $4::uuid IS NOT NULL AND $5::bool IS NOT NULL AND ({})",
            sql.trim_start_matches("SELECT ")
        );
        sqlx::query_scalar(&sql)
            .bind(org)
            .bind(target)
            .bind(vis)
            .bind(access.user_id.0)
            .bind(access.is_guest)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err)
    }
}
