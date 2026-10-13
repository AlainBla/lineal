//! `LabelRepo` für `PgTx` (WP06): Issue-, Projekt- und Initiative-Labels in drei Tabellen.

use super::org::conflict_or_internal;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::LabelRepo;
use lineal_app::ports::repos::label::{Label, LabelKind, LabelListFilter, LabelPatch, NewLabel};
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::LabelGroupType;
use lineal_domain::ids::{LabelId, TeamId, UserId};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

const fn table(kind: LabelKind) -> &'static str {
    match kind {
        LabelKind::Issue => "issue_label",
        LabelKind::Project => "project_label",
        LabelKind::Initiative => "initiative_label",
    }
}

/// Spaltenliste; fehlende Spalten der Familien werden als NULL geliefert.
fn cols(kind: LabelKind) -> String {
    let team = match kind {
        LabelKind::Initiative => "NULL::uuid AS team_id",
        _ => "team_id",
    };
    let group_type = match kind {
        LabelKind::Issue => "group_type",
        _ => "NULL::text AS group_type",
    };
    format!(
        "id, name, color, description, {team}, parent_id, is_group, {group_type}, inherited_from_id, \
         creator_id, retired_at, retired_by_id, last_applied_at, archived_at, version, created_at, updated_at"
    )
}

#[derive(sqlx::FromRow)]
struct LabelRow {
    id: Uuid,
    name: String,
    color: String,
    description: Option<String>,
    team_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    is_group: bool,
    group_type: Option<String>,
    inherited_from_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    retired_at: Option<DateTime<Utc>>,
    retired_by_id: Option<Uuid>,
    last_applied_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl LabelRow {
    fn into_label(self, kind: LabelKind) -> AppResult<Label> {
        let group_type = match self.group_type.as_deref() {
            None => None,
            Some(s) => Some(
                s.parse::<LabelGroupType>()
                    .map_err(|_| AppError::internal("invalid label group type in database"))?,
            ),
        };
        Ok(Label {
            id: LabelId(self.id),
            kind,
            name: self.name,
            color: self.color,
            description: self.description,
            team_id: self.team_id.map(TeamId),
            parent_id: self.parent_id.map(LabelId),
            is_group: self.is_group,
            group_type,
            inherited_from_id: self.inherited_from_id.map(LabelId),
            creator_id: self.creator_id.map(UserId),
            retired_at: self.retired_at,
            retired_by_id: self.retired_by_id.map(UserId),
            last_applied_at: self.last_applied_at,
            archived_at: self.archived_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const NAME_CONFLICT: &str = "A label with this name already exists in this scope.";

#[async_trait]
impl LabelRepo for PgTx {
    async fn insert(&mut self, kind: LabelKind, l: &NewLabel) -> AppResult<Label> {
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "INSERT INTO {} (id, organization_id, name, color, description, parent_id, is_group, creator_id, \
             created_at, updated_at",
            table(kind)
        ));
        if kind != LabelKind::Initiative {
            qb.push(", team_id");
        }
        if kind == LabelKind::Issue {
            qb.push(", group_type");
        }
        qb.push(") VALUES (");
        qb.push_bind(l.id.0).push(", ");
        qb.push_bind(self.org.0).push(", ");
        qb.push_bind(l.name.clone()).push(", ");
        qb.push_bind(l.color.clone()).push(", ");
        qb.push_bind(l.description.clone()).push(", ");
        qb.push_bind(l.parent_id.map(|p| p.0)).push(", ");
        qb.push_bind(l.is_group).push(", ");
        qb.push_bind(l.creator_id.map(|c| c.0)).push(", ");
        qb.push_bind(l.now).push(", ");
        qb.push_bind(l.now);
        if kind != LabelKind::Initiative {
            qb.push(", ");
            qb.push_bind(l.team_id.map(|t| t.0));
        }
        if kind == LabelKind::Issue {
            qb.push(", ");
            qb.push_bind(l.group_type.map(|g| g.as_str()));
        }
        qb.push(format!(") RETURNING {}", cols(kind)));
        let row = qb
            .build_query_as::<LabelRow>()
            .fetch_one(&mut *self.tx)
            .await
            .map_err(|e| conflict_or_internal(e, NAME_CONFLICT))?;
        row.into_label(kind)
    }

    async fn get(&mut self, kind: LabelKind, id: LabelId) -> AppResult<Option<Label>> {
        let row = sqlx::query_as::<_, LabelRow>(&format!(
            "SELECT {} FROM {} WHERE organization_id = $1 AND id = $2",
            cols(kind),
            table(kind)
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(|r| r.into_label(kind)).transpose()
    }

    async fn get_many(&mut self, kind: LabelKind, ids: &[LabelId]) -> AppResult<Vec<Label>> {
        let v: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        let rows = sqlx::query_as::<_, LabelRow>(&format!(
            "SELECT {} FROM {} WHERE organization_id = $1 AND id = ANY($2)",
            cols(kind),
            table(kind)
        ))
        .bind(self.org.0)
        .bind(v)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(|r| r.into_label(kind)).collect()
    }

    async fn find_by_name(&mut self, kind: LabelKind, name: &str) -> AppResult<Vec<Label>> {
        let rows = sqlx::query_as::<_, LabelRow>(&format!(
            "SELECT {} FROM {} WHERE organization_id = $1 AND lower(name) = lower($2) AND archived_at IS NULL \
             ORDER BY created_at, id",
            cols(kind),
            table(kind)
        ))
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(|r| r.into_label(kind)).collect()
    }

    async fn children(&mut self, kind: LabelKind, group: LabelId) -> AppResult<Vec<Label>> {
        let rows = sqlx::query_as::<_, LabelRow>(&format!(
            "SELECT {} FROM {} WHERE organization_id = $1 AND parent_id = $2 AND archived_at IS NULL \
             ORDER BY name, id",
            cols(kind),
            table(kind)
        ))
        .bind(self.org.0)
        .bind(group.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(|r| r.into_label(kind)).collect()
    }

    async fn list(
        &mut self,
        kind: LabelKind,
        f: &LabelListFilter,
        page: &PageRequest,
    ) -> AppResult<Page<Label>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "created_at",
            OrderBy::UpdatedAt => "updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {}, {order_col} AS sort_key FROM {} WHERE organization_id = ",
            cols(kind),
            table(kind)
        ));
        qb.push_bind(self.org.0);
        if kind != LabelKind::Initiative {
            let ws = f.scopes.contains(&None);
            let teams: Vec<Uuid> = f.scopes.iter().flatten().map(|t| t.0).collect();
            qb.push(" AND ((team_id IS NULL AND ");
            qb.push_bind(ws);
            qb.push(") OR team_id = ANY(");
            qb.push_bind(teams);
            qb.push("))");
        }
        if !f.include_groups {
            qb.push(" AND NOT is_group");
        }
        if !f.include_archived {
            qb.push(" AND archived_at IS NULL");
        }
        if let Some(n) = f.name.as_deref().filter(|n| !n.is_empty()) {
            let esc = n
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            qb.push(" AND lower(name) LIKE ");
            qb.push_bind(format!("%{esc}%"));
            qb.push(" ESCAPE '\\'");
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, id DESC LIMIT "));
        qb.push_bind(page.limit + 1);

        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            label: LabelRow,
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
                id: r.label.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.label.into_label(kind))
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn update(
        &mut self,
        kind: LabelKind,
        id: LabelId,
        p: &LabelPatch,
        now: DateTime<Utc>,
    ) -> AppResult<Label> {
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "UPDATE {} SET version = version + 1, updated_at = ",
            table(kind)
        ));
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
        if let Some(v) = &p.parent_id {
            qb.push(", parent_id = ").push_bind(v.map(|l| l.0));
        }
        if let Some(v) = p.is_group {
            qb.push(", is_group = ").push_bind(v);
        }
        if kind == LabelKind::Issue
            && let Some(v) = p.group_type
        {
            qb.push(", group_type = ").push_bind(v.as_str());
        }
        qb.push(" WHERE organization_id = ").push_bind(self.org.0);
        qb.push(" AND id = ").push_bind(id.0);
        qb.push(format!(" RETURNING {}", cols(kind)));
        let row = qb
            .build_query_as::<LabelRow>()
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|e| conflict_or_internal(e, NAME_CONFLICT))?;
        row.ok_or_else(|| AppError::not_found("Could not find referenced label."))?
            .into_label(kind)
    }

    async fn set_retired(
        &mut self,
        kind: LabelKind,
        ids: &[LabelId],
        retired_by: Option<UserId>,
        retired: bool,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let v: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        sqlx::query(&format!(
            "UPDATE {} SET retired_at = CASE WHEN $3 THEN COALESCE(retired_at, $5) ELSE NULL END, \
             retired_by_id = CASE WHEN $3 THEN COALESCE(retired_by_id, $4) ELSE NULL END, \
             version = version + 1, updated_at = $5 \
             WHERE organization_id = $1 AND id = ANY($2) AND ((retired_at IS NULL) = $3)",
            table(kind)
        ))
        .bind(self.org.0)
        .bind(v)
        .bind(retired)
        .bind(retired_by.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn count_children(&mut self, kind: LabelKind, group: LabelId) -> AppResult<i64> {
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {} WHERE organization_id = $1 AND parent_id = $2 AND archived_at IS NULL",
            table(kind)
        ))
        .bind(self.org.0)
        .bind(group.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn issues_with_issue_labels(
        &mut self,
        labels: &[LabelId],
    ) -> AppResult<Vec<(lineal_domain::ids::IssueId, TeamId)>> {
        let ids: Vec<Uuid> = labels.iter().map(|l| l.0).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT i.id, i.team_id FROM issue_to_label l \
             JOIN issue i ON i.organization_id = l.organization_id AND i.id = l.issue_id \
             WHERE l.organization_id = $1 AND l.label_id = ANY($2) ORDER BY i.id",
        )
        .bind(self.org.0)
        .bind(ids)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(i, t)| (lineal_domain::ids::IssueId(i), TeamId(t)))
            .collect())
    }

    async fn active_issues_with_issue_labels(
        &mut self,
        labels: &[LabelId],
    ) -> AppResult<Vec<(lineal_domain::ids::IssueId, TeamId)>> {
        let ids: Vec<Uuid> = labels.iter().map(|l| l.0).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT i.id, i.team_id FROM issue_to_label l \
             JOIN issue i ON i.organization_id = l.organization_id AND i.id = l.issue_id \
             WHERE l.organization_id = $1 AND l.label_id = ANY($2) AND NOT i.trashed ORDER BY i.id",
        )
        .bind(self.org.0)
        .bind(ids)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(i, t)| (lineal_domain::ids::IssueId(i), TeamId(t)))
            .collect())
    }

    async fn merge_issue_label(&mut self, source: LabelId, target: LabelId) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_to_label (organization_id, issue_id, label_id, single_select_group_id, created_at) \
             SELECT organization_id, issue_id, $3, NULL, created_at FROM issue_to_label \
             WHERE organization_id = $1 AND label_id = $2 ON CONFLICT DO NOTHING",
        )
        .bind(self.org.0)
        .bind(source.0)
        .bind(target.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE issue_label SET last_applied_at = now() \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(target.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        // IDs sind UUID-Strings und damit eindeutig: Textersetzung im JSON ist sicher.
        sqlx::query(
            "UPDATE template SET template_data = replace(template_data::text, $2, $3)::jsonb \
             WHERE organization_id = $1 AND type = 'issue' AND template_data::text LIKE '%' || $2 || '%'",
        )
        .bind(self.org.0)
        .bind(source.0.to_string())
        .bind(target.0.to_string())
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE custom_view SET filter_data = replace(filter_data::text, $2, $3)::jsonb \
             WHERE organization_id = $1 AND filter_data::text LIKE '%' || $2 || '%'",
        )
        .bind(self.org.0)
        .bind(source.0.to_string())
        .bind(target.0.to_string())
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_issue_labels(&mut self, labels: &[LabelId]) -> AppResult<()> {
        let ids: Vec<Uuid> = labels.iter().map(|l| l.0).collect();
        sqlx::query(
            "UPDATE issue_label SET inherited_from_id = NULL \
             WHERE organization_id = $1 AND inherited_from_id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(&ids)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query("DELETE FROM issue_label WHERE organization_id = $1 AND parent_id = ANY($2)")
            .bind(self.org.0)
            .bind(&ids)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM issue_label WHERE organization_id = $1 AND id = ANY($2)")
            .bind(self.org.0)
            .bind(&ids)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}
