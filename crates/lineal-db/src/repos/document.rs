//! `DocumentRepo` für `PgTx` (WP10): Dokumente, `document_content` und Snapshot-Historie.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::DocumentRepo;
use lineal_app::ports::repos::document::{
    DocParent, Document, DocumentQuery, HistoryEntry, HistoryKind, NewDocument, NewHistory,
    ParentScope, ParentState,
};
use lineal_app::ports::repos::user::OrderBy;
use lineal_domain::ids::*;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct DocRow {
    id: Uuid,
    slug_id: String,
    title: String,
    content: String,
    content_id: Option<Uuid>,
    content_version: i32,
    restored_at: Option<DateTime<Utc>>,
    icon: Option<String>,
    color: Option<String>,
    project_id: Option<Uuid>,
    issue_id: Option<Uuid>,
    initiative_id: Option<Uuid>,
    cycle_id: Option<Uuid>,
    team_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    updated_by_id: Option<Uuid>,
    sort_order: f64,
    last_applied_template_id: Option<Uuid>,
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    trashed_with_parent: bool,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
}

/// Spalten von `document d` plus Inhalt (`c` = `document_content`).
const COLS: &str = "d.id, d.slug_id, d.title, COALESCE(c.markdown, d.content, '') AS content, \
    c.id AS content_id, COALESCE(c.version, 1) AS content_version, c.restored_at, d.icon, d.color, \
    d.project_id, d.issue_id, d.initiative_id, d.cycle_id, d.team_id, d.creator_id, \
    d.updated_by_id, d.sort_order, d.last_applied_template_id, d.trashed, d.trashed_at, \
    d.trashed_with_parent, d.version, d.created_at, d.updated_at, d.archived_at";

const FROM: &str = "FROM document d LEFT JOIN document_content c ON c.organization_id = \
    d.organization_id AND c.owner_type = 'document' AND c.owner_id = d.id";

impl DocRow {
    fn into_doc(self, content_id: Uuid) -> Document {
        Document {
            id: DocumentId(self.id),
            slug_id: self.slug_id,
            title: self.title,
            content: self.content,
            content_id,
            content_version: self.content_version,
            restored_at: self.restored_at,
            icon: self.icon,
            color: self.color,
            project_id: self.project_id.map(ProjectId),
            issue_id: self.issue_id.map(IssueId),
            initiative_id: self.initiative_id.map(InitiativeId),
            cycle_id: self.cycle_id.map(CycleId),
            team_id: self.team_id.map(TeamId),
            creator_id: self.creator_id.map(UserId),
            updated_by_id: self.updated_by_id.map(UserId),
            sort_order: self.sort_order,
            last_applied_template_id: self.last_applied_template_id.map(TemplateId),
            trashed: self.trashed,
            trashed_at: self.trashed_at,
            trashed_with_parent: self.trashed_with_parent,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct HistRow {
    id: Uuid,
    document_content_id: Uuid,
    markdown: String,
    title: Option<String>,
    actor_ids: Vec<Uuid>,
    content_snapshot_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    kind: String,
    restored_from_id: Option<Uuid>,
}

const HIST_COLS: &str = "id, document_content_id, markdown, title, actor_ids, content_snapshot_at, \
    created_at, kind, restored_from_id";

impl From<HistRow> for HistoryEntry {
    fn from(r: HistRow) -> Self {
        Self {
            id: r.id,
            document_content_id: r.document_content_id,
            markdown: r.markdown,
            title: r.title,
            actor_ids: r.actor_ids.into_iter().map(UserId).collect(),
            content_snapshot_at: r.content_snapshot_at,
            created_at: r.created_at,
            kind: HistoryKind::parse(&r.kind).unwrap_or(HistoryKind::Edited),
            restored_from_id: r.restored_from_id,
        }
    }
}

/// Spalte und Wert des Elternobjekts für Filter.
fn parent_col(p: DocParent) -> (&'static str, Uuid) {
    match p {
        DocParent::Project(i) => ("project_id", i.0),
        DocParent::Issue(i) => ("issue_id", i.0),
        DocParent::Initiative(i) => ("initiative_id", i.0),
        DocParent::Cycle { cycle, .. } => ("cycle_id", cycle.0),
        DocParent::Team(t) => ("team_id", t.0),
    }
}

/// Escaped `%`, `_` und `\` für `ILIKE ... ESCAPE '\'`.
fn like_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('%');
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl PgTx {
    async fn doc_by(&mut self, filter: &str, bind: Uuid) -> AppResult<Option<Document>> {
        self.doc_query(filter, Bind::Uuid(bind)).await
    }

    async fn doc_query(&mut self, filter: &str, bind: Bind) -> AppResult<Option<Document>> {
        let sql = format!("SELECT {COLS} {FROM} WHERE d.organization_id = $1 AND {filter}");
        let q = sqlx::query_as::<_, DocRow>(&sql).bind(self.org.0);
        let q = match bind {
            Bind::Uuid(u) => q.bind(u),
            Bind::Text(t) => q.bind(t),
        };
        let row = q.fetch_optional(&mut *self.tx).await.map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        // Dokumente ohne Inhaltszeile (z. B. aus Importen) bekommen sie nachträglich.
        let content_id = match row.content_id {
            Some(c) => c,
            None => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO document_content (id, organization_id, owner_type, owner_id, markdown) \
                     VALUES ($1, $2, 'document', $3, $4) ON CONFLICT DO NOTHING",
                )
                .bind(id)
                .bind(self.org.0)
                .bind(row.id)
                .bind(&row.content)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
                id
            }
        };
        Ok(Some(row.into_doc(content_id)))
    }
}

enum Bind {
    Uuid(Uuid),
    Text(String),
}

#[async_trait]
impl DocumentRepo for PgTx {
    async fn insert(&mut self, d: &NewDocument) -> AppResult<Document> {
        let mut doc = Document {
            id: d.id,
            slug_id: String::new(),
            title: d.title.clone(),
            content: d.markdown.clone(),
            content_id: d.content_id,
            content_version: 1,
            restored_at: None,
            icon: d.icon.clone(),
            color: d.color.clone(),
            project_id: None,
            issue_id: None,
            initiative_id: None,
            cycle_id: None,
            team_id: None,
            creator_id: d.creator_id,
            updated_by_id: d.creator_id,
            sort_order: d.sort_order,
            last_applied_template_id: d.last_applied_template_id,
            trashed: false,
            trashed_at: None,
            trashed_with_parent: false,
            version: 1,
            created_at: d.now,
            updated_at: d.now,
            archived_at: None,
        };
        doc.set_parent(d.parent);
        sqlx::query(
            "INSERT INTO document (id, organization_id, title, content, icon, color, project_id, issue_id, \
             initiative_id, cycle_id, team_id, creator_id, updated_by_id, sort_order, \
             last_applied_template_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12, $13, $14, $15, $15)",
        )
        .bind(d.id.0)
        .bind(self.org.0)
        .bind(&d.title)
        .bind(&d.markdown)
        .bind(&d.icon)
        .bind(&d.color)
        .bind(doc.project_id.map(|i| i.0))
        .bind(doc.issue_id.map(|i| i.0))
        .bind(doc.initiative_id.map(|i| i.0))
        .bind(doc.cycle_id.map(|i| i.0))
        .bind(doc.team_id.map(|i| i.0))
        .bind(d.creator_id.map(|u| u.0))
        .bind(d.sort_order)
        .bind(d.last_applied_template_id.map(|t| t.0))
        .bind(d.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "INSERT INTO document_content (id, organization_id, owner_type, owner_id, markdown, \
             created_at, updated_at) VALUES ($1, $2, 'document', $3, $4, $5, $5)",
        )
        .bind(d.content_id)
        .bind(self.org.0)
        .bind(d.id.0)
        .bind(&d.markdown)
        .bind(d.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        self.doc_by("d.id = $2", d.id.0)
            .await?
            .ok_or_else(|| lineal_app::AppError::internal("document missing after insert"))
    }

    async fn get(&mut self, id: DocumentId) -> AppResult<Option<Document>> {
        self.doc_by("d.id = $2", id.0).await
    }

    async fn find_by_slug_id(&mut self, slug_id: &str) -> AppResult<Option<Document>> {
        self.doc_query("d.slug_id = $2", Bind::Text(slug_id.to_owned()))
            .await
    }

    async fn update(
        &mut self,
        d: &Document,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Document> {
        sqlx::query(
            "UPDATE document SET title = $3, icon = $4, color = $5, project_id = $6, issue_id = $7, \
             initiative_id = $8, cycle_id = $9, team_id = $10, sort_order = $11, \
             last_applied_template_id = $12, trashed = $13, trashed_at = $14, \
             trashed_with_parent = $15, updated_by_id = $16, updated_at = $17, version = version + 1 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(d.id.0)
        .bind(&d.title)
        .bind(&d.icon)
        .bind(&d.color)
        .bind(d.project_id.map(|i| i.0))
        .bind(d.issue_id.map(|i| i.0))
        .bind(d.initiative_id.map(|i| i.0))
        .bind(d.cycle_id.map(|i| i.0))
        .bind(d.team_id.map(|i| i.0))
        .bind(d.sort_order)
        .bind(d.last_applied_template_id.map(|t| t.0))
        .bind(d.trashed)
        .bind(d.trashed_at)
        .bind(d.trashed_with_parent)
        .bind(updated_by.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        self.doc_by("d.id = $2", d.id.0)
            .await?
            .ok_or_else(|| lineal_app::AppError::not_found("Could not find referenced Document."))
    }

    async fn set_content(
        &mut self,
        id: DocumentId,
        markdown: &str,
        restored: bool,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<i32> {
        let version: Option<i32> = sqlx::query_scalar(
            "UPDATE document_content SET markdown = $3, version = version + 1, updated_at = $4, \
             restored_at = CASE WHEN $5 THEN $4 ELSE restored_at END \
             WHERE organization_id = $1 AND owner_type = 'document' AND owner_id = $2 RETURNING version",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(markdown)
        .bind(now)
        .bind(restored)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let version = version
            .ok_or_else(|| lineal_app::AppError::not_found("Could not find document content."))?;
        sqlx::query(
            "UPDATE document SET content = $3, updated_by_id = $4, updated_at = $5 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(markdown)
        .bind(updated_by.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(version)
    }

    async fn max_sort_order(&mut self, parent: DocParent) -> AppResult<Option<f64>> {
        let (col, id) = parent_col(parent);
        sqlx::query_scalar(&format!(
            "SELECT max(sort_order) FROM document WHERE organization_id = $1 AND {col} = $2 \
             AND NOT trashed"
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn list(&mut self, q: &DocumentQuery) -> AppResult<Vec<Document>> {
        let key = match q.order_by {
            OrderBy::CreatedAt => "d.created_at",
            OrderBy::UpdatedAt => "d.updated_at",
        };
        let visible: Vec<Uuid> = q.visible_teams.iter().map(|t| t.0).collect();
        let mut b: QueryBuilder<'_, Postgres> =
            QueryBuilder::new(format!("SELECT {COLS} {FROM} WHERE d.organization_id = "));
        b.push_bind(self.org.0);
        b.push(" AND d.trashed = ").push_bind(q.trashed);
        // Sichtbarkeit über das Elternobjekt (zur Laufzeit abgeleitet, docs/06 §1.7).
        b.push(" AND (d.team_id = ANY(");
        b.push_bind(visible.clone());
        b.push(
            ") OR d.issue_id IN (SELECT i.id FROM issue i WHERE i.organization_id = d.organization_id \
             AND (i.team_id = ANY(",
        );
        b.push_bind(visible.clone());
        b.push(")");
        if let Some(u) = q.actor {
            b.push(
                " OR EXISTS (SELECT 1 FROM issue_share s WHERE s.organization_id = i.organization_id \
                 AND s.issue_id = i.id AND s.user_id = ",
            );
            b.push_bind(u.0);
            b.push(")");
        }
        b.push("))");
        b.push(
            " OR d.project_id IN (SELECT pt.project_id FROM project_team pt WHERE \
             pt.organization_id = d.organization_id AND pt.team_id = ANY(",
        );
        b.push_bind(visible.clone());
        b.push("))");
        if q.initiatives_visible {
            b.push(
                " OR d.initiative_id IN (SELECT n.id FROM initiative n WHERE n.organization_id = \
                 d.organization_id AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY(",
            );
            b.push_bind(visible);
            b.push(")))");
        }
        b.push(")");
        if !q.include_archived && !q.trashed {
            b.push(
                " AND d.archived_at IS NULL \
                 AND (d.issue_id IS NULL OR EXISTS (SELECT 1 FROM issue i WHERE i.organization_id = d.organization_id \
                   AND i.id = d.issue_id AND i.archived_at IS NULL AND NOT i.trashed)) \
                 AND (d.project_id IS NULL OR EXISTS (SELECT 1 FROM project p WHERE p.organization_id = d.organization_id \
                   AND p.id = d.project_id AND p.archived_at IS NULL AND NOT p.trashed)) \
                 AND (d.initiative_id IS NULL OR EXISTS (SELECT 1 FROM initiative n WHERE n.organization_id = d.organization_id \
                   AND n.id = d.initiative_id AND n.archived_at IS NULL AND NOT n.trashed)) \
                 AND (d.cycle_id IS NULL OR EXISTS (SELECT 1 FROM cycle y WHERE y.organization_id = d.organization_id \
                   AND y.id = d.cycle_id AND y.archived_at IS NULL))",
            );
        }
        if let Some(t) = &q.text {
            let pattern = like_pattern(t);
            b.push(" AND (d.title ILIKE ");
            b.push_bind(pattern.clone());
            b.push(" ESCAPE '\\' OR COALESCE(c.markdown, d.content, '') ILIKE ");
            b.push_bind(pattern);
            b.push(" ESCAPE '\\')");
        }
        if let Some(u) = q.creator {
            b.push(" AND d.creator_id = ").push_bind(u.0);
        }
        if let Some(p) = q.project {
            b.push(" AND d.project_id = ").push_bind(p.0);
        }
        if let Some(i) = q.initiative {
            b.push(" AND d.initiative_id = ").push_bind(i.0);
        }
        if let Some(i) = q.issue {
            b.push(" AND d.issue_id = ").push_bind(i.0);
        }
        if let Some(c) = q.cycle {
            b.push(" AND d.cycle_id = ").push_bind(c.0);
        }
        if let Some(t) = q.team {
            b.push(" AND (d.team_id = ").push_bind(t.0);
            b.push(
                " OR d.issue_id IN (SELECT i.id FROM issue i WHERE i.organization_id = d.organization_id AND i.team_id = ",
            );
            b.push_bind(t.0);
            b.push(
                ") OR d.project_id IN (SELECT pt.project_id FROM project_team pt WHERE \
                 pt.organization_id = d.organization_id AND pt.team_id = ",
            );
            b.push_bind(t.0);
            b.push("))");
        }
        if let Some(t) = q.created_after {
            b.push(" AND d.created_at > ").push_bind(t);
        }
        if let Some(t) = q.updated_after {
            b.push(" AND d.updated_at > ").push_bind(t);
        }
        if let Some(c) = &q.cursor {
            b.push(format!(" AND ({key}, d.id) < ("));
            b.push_bind(c.key);
            b.push(", ");
            b.push_bind(c.id);
            b.push(")");
        }
        b.push(format!(" ORDER BY {key} DESC, d.id DESC LIMIT "));
        b.push_bind(q.fetch);
        let rows = b
            .build_query_as::<DocRow>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let cid = r.content_id.unwrap_or_else(Uuid::nil);
            out.push(r.into_doc(cid));
        }
        Ok(out)
    }

    async fn parent_scope(&mut self, parent: DocParent) -> AppResult<Option<ParentScope>> {
        let org = self.org.0;
        match parent {
            DocParent::Issue(i) => {
                let row: Option<(Uuid, bool, bool)> = sqlx::query_as(
                    "SELECT team_id, archived_at IS NOT NULL, trashed FROM issue \
                     WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(i.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(row.map(|(t, archived, trashed)| ParentScope {
                    teams: vec![TeamId(t)],
                    workspace: false,
                    state: ParentState { archived, trashed },
                }))
            }
            DocParent::Project(p) => {
                let row: Option<(bool, bool)> = sqlx::query_as(
                    "SELECT archived_at IS NOT NULL, trashed FROM project \
                     WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(p.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                let Some((archived, trashed)) = row else {
                    return Ok(None);
                };
                let teams: Vec<Uuid> = sqlx::query_scalar(
                    "SELECT team_id FROM project_team WHERE organization_id = $1 AND project_id = $2",
                )
                .bind(org)
                .bind(p.0)
                .fetch_all(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(Some(ParentScope {
                    teams: teams.into_iter().map(TeamId).collect(),
                    workspace: false,
                    state: ParentState { archived, trashed },
                }))
            }
            DocParent::Initiative(i) => {
                let row: Option<(Option<Uuid>, bool, bool)> = sqlx::query_as(
                    "SELECT lead_team_id, archived_at IS NOT NULL, trashed FROM initiative \
                     WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(i.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(row.map(|(t, archived, trashed)| ParentScope {
                    teams: t.map(TeamId).into_iter().collect(),
                    workspace: true,
                    state: ParentState { archived, trashed },
                }))
            }
            DocParent::Cycle { cycle, .. } => {
                let row: Option<(Uuid, bool)> = sqlx::query_as(
                    "SELECT team_id, archived_at IS NOT NULL FROM cycle \
                     WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(cycle.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(row.map(|(t, archived)| ParentScope {
                    teams: vec![TeamId(t)],
                    workspace: false,
                    state: ParentState {
                        archived,
                        trashed: false,
                    },
                }))
            }
            DocParent::Team(t) => {
                let row: Option<bool> = sqlx::query_scalar(
                    "SELECT trashed FROM team WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(t.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(row.map(|trashed| ParentScope {
                    teams: vec![t],
                    workspace: false,
                    state: ParentState {
                        archived: false,
                        trashed,
                    },
                }))
            }
        }
    }

    async fn issue_shared_with(&mut self, issue: IssueId, user: UserId) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM issue_share WHERE organization_id = $1 AND issue_id = $2 \
             AND user_id = $3)",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn subscribe(
        &mut self,
        id: Uuid,
        document: DocumentId,
        user: UserId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO document_subscription (id, organization_id, document_id, user_id, reason, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) \
             ON CONFLICT (document_id, user_id) DO NOTHING",
        )
        .bind(id)
        .bind(self.org.0)
        .bind(document.0)
        .bind(user.0)
        .bind(reason)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn subscribers(&mut self, document: DocumentId) -> AppResult<Vec<UserId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM document_subscription WHERE organization_id = $1 \
             AND document_id = $2 AND active AND archived_at IS NULL ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(document.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }

    async fn history_insert(&mut self, h: &NewHistory) -> AppResult<HistoryEntry> {
        let actors: Vec<Uuid> = h.actor_ids.iter().map(|u| u.0).collect();
        let row = sqlx::query_as::<_, HistRow>(&format!(
            "INSERT INTO document_content_history (id, organization_id, document_content_id, markdown, \
             title, actor_ids, content_snapshot_at, kind, restored_from_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {HIST_COLS}"
        ))
        .bind(h.id)
        .bind(self.org.0)
        .bind(h.document_content_id)
        .bind(&h.markdown)
        .bind(&h.title)
        .bind(actors)
        .bind(h.now)
        .bind(h.kind.as_str())
        .bind(h.restored_from_id)
        .bind(h.created_at.unwrap_or(h.now))
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.into())
    }

    async fn history_latest(&mut self, content_id: Uuid) -> AppResult<Option<HistoryEntry>> {
        let row = sqlx::query_as::<_, HistRow>(&format!(
            "SELECT {HIST_COLS} FROM document_content_history WHERE organization_id = $1 \
             AND document_content_id = $2 ORDER BY content_snapshot_at DESC, id DESC LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(content_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn history_list(
        &mut self,
        content_id: Uuid,
        before: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> AppResult<Vec<HistoryEntry>> {
        let mut b: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {HIST_COLS} FROM document_content_history WHERE organization_id = "
        ));
        b.push_bind(self.org.0);
        b.push(" AND document_content_id = ").push_bind(content_id);
        if let Some((ts, id)) = before {
            b.push(" AND (content_snapshot_at, id) < (");
            b.push_bind(ts);
            b.push(", ");
            b.push_bind(id);
            b.push(")");
        }
        b.push(" ORDER BY content_snapshot_at DESC, id DESC LIMIT ")
            .push_bind(limit);
        let rows = b
            .build_query_as::<HistRow>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn history_get(&mut self, content_id: Uuid, id: Uuid) -> AppResult<Option<HistoryEntry>> {
        let row = sqlx::query_as::<_, HistRow>(&format!(
            "SELECT {HIST_COLS} FROM document_content_history WHERE organization_id = $1 \
             AND document_content_id = $2 AND id = $3"
        ))
        .bind(self.org.0)
        .bind(content_id)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn cascade_trash(
        &mut self,
        parent: DocParent,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<DocumentId>> {
        let (col, id) = parent_col(parent);
        let rows: Vec<Uuid> = sqlx::query_scalar(&format!(
            "UPDATE document SET trashed = true, trashed_at = $3, trashed_with_parent = true, \
             updated_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND {col} = $2 AND NOT trashed RETURNING id"
        ))
        .bind(self.org.0)
        .bind(id)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(DocumentId).collect())
    }

    async fn cascade_restore(&mut self, parent: DocParent) -> AppResult<Vec<DocumentId>> {
        let (col, id) = parent_col(parent);
        let rows: Vec<Uuid> = sqlx::query_scalar(&format!(
            "UPDATE document SET trashed = false, trashed_at = NULL, trashed_with_parent = false, \
             version = version + 1 \
             WHERE organization_id = $1 AND {col} = $2 AND trashed_with_parent RETURNING id"
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(DocumentId).collect())
    }

    async fn purge_trashed(
        &mut self,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<DocumentId>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM document WHERE organization_id = $1 AND id IN ( \
               SELECT id FROM document WHERE organization_id = $1 AND trashed \
                 AND trashed_at IS NOT NULL AND trashed_at <= $2 ORDER BY trashed_at, id LIMIT $3) \
             RETURNING id",
        )
        .bind(self.org.0)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        self.sweep_orphan_contents(limit.saturating_mul(4)).await?;
        Ok(ids.into_iter().map(DocumentId).collect())
    }

    async fn sweep_orphan_contents(&mut self, limit: i64) -> AppResult<u64> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM document_content c WHERE c.organization_id = $1 \
               AND c.owner_type = 'document' \
               AND NOT EXISTS (SELECT 1 FROM document d WHERE d.organization_id = c.organization_id \
                               AND d.id = c.owner_id) \
             LIMIT $2",
        )
        .bind(self.org.0)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if ids.is_empty() {
            return Ok(0);
        }
        sqlx::query(
            "DELETE FROM document_content_history WHERE organization_id = $1 \
               AND document_content_id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(&ids)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let n =
            sqlx::query("DELETE FROM document_content WHERE organization_id = $1 AND id = ANY($2)")
                .bind(self.org.0)
                .bind(&ids)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?
                .rows_affected();
        Ok(n)
    }

    async fn history_delete(&mut self, content_id: Uuid, id: Uuid) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM document_content_history WHERE organization_id = $1 \
             AND document_content_id = $2 AND id = $3",
        )
        .bind(self.org.0)
        .bind(content_id)
        .bind(id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
