//! `CommentRepo` für `PgTx` (WP09): Kommentare, Reaktionen, Eltern-Auflösung, Abonnements.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::CommentRepo;
use lineal_app::ports::repos::comment::{
    Comment, CommentQuery, MentionCandidate, NewComment, ParentKind, ParentRef, ParentScope,
    Reaction, ReactionTarget,
};
use lineal_app::ports::repos::user::OrderBy;
use lineal_app::{AppError, AppResult};
use lineal_domain::ids::*;
use serde_json::json;
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct CommentRow {
    id: Uuid,
    user_id: Option<Uuid>,
    body: String,
    issue_id: Option<Uuid>,
    project_id: Option<Uuid>,
    project_milestone_id: Option<Uuid>,
    project_update_id: Option<Uuid>,
    initiative_id: Option<Uuid>,
    initiative_update_id: Option<Uuid>,
    document_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    quoted_text: Option<String>,
    anchor_start: Option<String>,
    anchor_end: Option<String>,
    edited_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
    resolving_comment_id: Option<Uuid>,
    resolving_user_id: Option<Uuid>,
    deleted_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
}

const COMMENT_COLS: &str = "id, user_id, body, issue_id, project_id, project_milestone_id, \
    project_update_id, initiative_id, initiative_update_id, document_id, parent_id, quoted_text, \
    anchor_start, anchor_end, edited_at, resolved_at, resolving_comment_id, resolving_user_id, deleted_at, version, \
    created_at, updated_at, archived_at";

impl CommentRow {
    fn into_comment(self) -> AppResult<Comment> {
        let parent = if let Some(i) = self.issue_id {
            ParentRef::Issue(IssueId(i))
        } else if let Some(i) = self.project_id {
            ParentRef::Project(ProjectId(i))
        } else if let Some(i) = self.project_milestone_id {
            ParentRef::Milestone(MilestoneId(i))
        } else if let Some(i) = self.project_update_id {
            ParentRef::ProjectUpdate(ProjectUpdateId(i))
        } else if let Some(i) = self.initiative_id {
            ParentRef::Initiative(InitiativeId(i))
        } else if let Some(i) = self.initiative_update_id {
            ParentRef::InitiativeUpdate(InitiativeUpdateId(i))
        } else if let Some(i) = self.document_id {
            ParentRef::Document(DocumentId(i))
        } else {
            return Err(AppError::internal("comment without parent"));
        };
        Ok(Comment {
            id: CommentId(self.id),
            user_id: self.user_id.map(UserId),
            body: self.body,
            parent,
            parent_id: self.parent_id.map(CommentId),
            quoted_text: self.quoted_text,
            anchor_start: self.anchor_start,
            anchor_end: self.anchor_end,
            edited_at: self.edited_at,
            resolved_at: self.resolved_at,
            resolving_comment_id: self.resolving_comment_id.map(CommentId),
            resolving_user_id: self.resolving_user_id.map(UserId),
            deleted_at: self.deleted_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived_at: self.archived_at,
        })
    }
}

/// Spaltenname des Elternobjekts in `comment`.
fn parent_col(kind: ParentKind) -> &'static str {
    match kind {
        ParentKind::Issue => "issue_id",
        ParentKind::Project => "project_id",
        ParentKind::Initiative => "initiative_id",
        ParentKind::Document => "document_id",
        ParentKind::Milestone => "project_milestone_id",
        ParentKind::ProjectUpdate => "project_update_id",
        ParentKind::InitiativeUpdate => "initiative_update_id",
    }
}

fn ambiguous(reference: &str, kind: &str) -> AppError {
    AppError::ambiguous_reference(format!("{kind} \"{reference}\" matches several objects."))
        .with_details(json!({ "reference": reference, "type": kind }))
}

impl PgTx {
    async fn one_or_ambiguous(
        &mut self,
        sql: &str,
        reference: &str,
        extra: Option<i32>,
        kind: &str,
    ) -> AppResult<Option<Uuid>> {
        let mut q = sqlx::query_scalar::<_, Uuid>(sql)
            .bind(self.org.0)
            .bind(reference.to_owned());
        if let Some(n) = extra {
            q = q.bind(n);
        }
        let mut rows = q.fetch_all(&mut *self.tx).await.map_err(db_err)?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(rows.pop()),
            _ => Err(ambiguous(reference, kind)),
        }
    }

    /// Issue per UUID, Kennung oder früherer Kennung (nicht im Papierkorb).
    pub(crate) async fn find_issue_id(&mut self, reference: &str) -> AppResult<Option<Uuid>> {
        let reference = reference.trim();
        if let Ok(id) = Uuid::parse_str(reference) {
            return sqlx::query_scalar(
                "SELECT id FROM issue WHERE organization_id = $1 AND id = $2 AND NOT trashed",
            )
            .bind(self.org.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err);
        }
        let found: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND upper(identifier) = upper($2) \
             AND NOT trashed ORDER BY created_at DESC LIMIT 1",
        )
        .bind(self.org.0)
        .bind(reference)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if found.is_some() {
            return Ok(found);
        }
        sqlx::query_scalar(
            "SELECT a.issue_id FROM issue_identifier_alias a \
             JOIN issue i ON i.organization_id = a.organization_id AND i.id = a.issue_id \
             WHERE a.organization_id = $1 AND a.identifier = upper($2) AND a.entity_type = 'issue' \
               AND a.archived_at IS NULL AND NOT i.trashed",
        )
        .bind(self.org.0)
        .bind(reference)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn project_team_ids(&mut self, project: Uuid) -> AppResult<Vec<TeamId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT lead_team_id FROM project WHERE organization_id = $1 AND id = $2 AND NOT trashed \
             UNION SELECT pt.team_id FROM project_team pt \
               JOIN project p ON p.organization_id = pt.organization_id AND p.id = pt.project_id \
               WHERE pt.organization_id = $1 AND pt.project_id = $2 AND NOT p.trashed",
        )
        .bind(self.org.0)
        .bind(project)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(TeamId).collect())
    }

    async fn exists(&mut self, sql: &str, id: Uuid) -> AppResult<bool> {
        let n: Option<i32> = sqlx::query_scalar(sql)
            .bind(self.org.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(n.is_some())
    }
}

/// `P-ENG-7` → (`ENG`, 7); `I-4` → ("", 4).
fn split_identifier(reference: &str, prefix: &str) -> Option<(String, i32)> {
    let rest = reference.get(..prefix.len()).and_then(|p| {
        p.eq_ignore_ascii_case(prefix)
            .then(|| &reference[prefix.len()..])
    })?;
    if prefix == "I-" {
        return rest.parse().ok().map(|n| (String::new(), n));
    }
    let (key, n) = rest.rsplit_once('-')?;
    Some((key.to_ascii_uppercase(), n.parse().ok()?))
}

/// Slug-Endstück (12 Hex-Zeichen) einer Referenz wie `titel-0123456789ab`.
fn slug_of(reference: &str) -> Option<&str> {
    let tail = reference.rsplit('-').next().unwrap_or(reference);
    (tail.len() == 12 && tail.chars().all(|c| c.is_ascii_hexdigit())).then_some(tail)
}

#[async_trait]
impl CommentRepo for PgTx {
    async fn find_parent(
        &mut self,
        kind: ParentKind,
        reference: &str,
    ) -> AppResult<Option<ParentRef>> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Ok(None);
        }
        let id = match kind {
            ParentKind::Issue => self.find_issue_id(reference).await?,
            ParentKind::Milestone => match Uuid::parse_str(reference) {
                Ok(id) => self
                    .exists(
                        "SELECT 1 FROM project_milestone WHERE organization_id = $1 AND id = $2 \
                         AND archived_at IS NULL",
                        id,
                    )
                    .await?
                    .then_some(id),
                Err(_) => None,
            },
            ParentKind::Project => self.find_named(reference, "project", "Project").await?,
            ParentKind::Initiative => {
                self.find_named(reference, "initiative", "Initiative")
                    .await?
            }
            ParentKind::Document => {
                if let Ok(id) = Uuid::parse_str(reference) {
                    self.exists(
                        "SELECT 1 FROM document WHERE organization_id = $1 AND id = $2 AND NOT trashed",
                        id,
                    )
                    .await?
                    .then_some(id)
                } else if let Some(slug) = slug_of(reference) {
                    sqlx::query_scalar(
                        "SELECT id FROM document WHERE organization_id = $1 AND slug_id = $2 AND NOT trashed",
                    )
                    .bind(self.org.0)
                    .bind(slug.to_ascii_lowercase())
                    .fetch_optional(&mut *self.tx)
                    .await
                    .map_err(db_err)?
                } else {
                    None
                }
            }
            ParentKind::ProjectUpdate | ParentKind::InitiativeUpdate => {
                let table = if kind == ParentKind::ProjectUpdate {
                    "project_update"
                } else {
                    "initiative_update"
                };
                if let Ok(id) = Uuid::parse_str(reference) {
                    self.exists(
                        &format!(
                            "SELECT 1 FROM {table} WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
                        ),
                        id,
                    )
                    .await?
                    .then_some(id)
                } else if let Some(slug) = slug_of(reference) {
                    sqlx::query_scalar(&format!(
                        "SELECT id FROM {table} WHERE organization_id = $1 AND slug_id = $2 AND archived_at IS NULL"
                    ))
                    .bind(self.org.0)
                    .bind(slug.to_ascii_lowercase())
                    .fetch_optional(&mut *self.tx)
                    .await
                    .map_err(db_err)?
                } else {
                    None
                }
            }
        };
        Ok(id.map(|id| ParentRef::from_parts(kind, id)))
    }

    async fn parent_scope(&mut self, parent: ParentRef) -> AppResult<Option<ParentScope>> {
        let id = parent.uuid();
        match parent {
            ParentRef::Issue(_) => {
                let team: Option<Uuid> = sqlx::query_scalar(
                    "SELECT team_id FROM issue WHERE id = $1 AND organization_id = $2 AND NOT trashed",
                )
                .bind(id)
                .bind(self.org.0)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                Ok(team.map(|t| ParentScope::Teams(vec![TeamId(t)])))
            }
            ParentRef::Project(_) => self.project_scope(id).await,
            ParentRef::Milestone(_) | ParentRef::ProjectUpdate(_) => {
                let table = if matches!(parent, ParentRef::Milestone(_)) {
                    "project_milestone"
                } else {
                    "project_update"
                };
                let project: Option<Uuid> = sqlx::query_scalar(&format!(
                    "SELECT project_id FROM {table} WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
                ))
                .bind(self.org.0)
                .bind(id)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(db_err)?;
                match project {
                    Some(p) => self.project_scope(p).await,
                    None => Ok(None),
                }
            }
            ParentRef::Initiative(_) => Ok(self
                .exists(
                    "SELECT 1 FROM initiative WHERE organization_id = $1 AND id = $2 AND NOT trashed",
                    id,
                )
                .await?
                .then_some(ParentScope::Workspace)),
            ParentRef::InitiativeUpdate(_) => Ok(self
                .exists(
                    "SELECT 1 FROM initiative_update WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
                    id,
                )
                .await?
                .then_some(ParentScope::Workspace)),
            ParentRef::Document(_) => {
                let row: Option<(Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>)> =
                    sqlx::query_as(
                        "SELECT issue_id, project_id, initiative_id, team_id FROM document \
                         WHERE organization_id = $1 AND id = $2 AND NOT trashed",
                    )
                    .bind(self.org.0)
                    .bind(id)
                    .fetch_optional(&mut *self.tx)
                    .await
                    .map_err(db_err)?;
                let Some((issue, project, initiative, team)) = row else {
                    return Ok(None);
                };
                if let Some(i) = issue {
                    return self.parent_scope(ParentRef::Issue(IssueId(i))).await;
                }
                if let Some(p) = project {
                    return self.project_scope(p).await;
                }
                if initiative.is_some() {
                    return Ok(Some(ParentScope::Workspace));
                }
                Ok(team.map(|t| ParentScope::Teams(vec![TeamId(t)])))
            }
        }
    }

    async fn issue_identifier(&mut self, issue: IssueId) -> AppResult<Option<String>> {
        sqlx::query_scalar("SELECT identifier FROM issue WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(issue.0)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)
    }

    async fn first_root(&mut self, parent: ParentRef) -> AppResult<Option<CommentId>> {
        let col = parent_col(parent.kind());
        let id: Option<Uuid> = sqlx::query_scalar(&format!(
            "SELECT id FROM comment WHERE organization_id = $1 AND {col} = $2 \
             AND parent_id IS NULL AND archived_at IS NULL ORDER BY created_at, id LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(parent.uuid())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id.map(CommentId))
    }

    async fn insert(&mut self, c: &NewComment) -> AppResult<Comment> {
        let col = parent_col(c.parent.kind());
        let row = sqlx::query_as::<_, CommentRow>(&format!(
            "INSERT INTO comment (id, organization_id, body, user_id, {col}, parent_id, quoted_text, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8) RETURNING {COMMENT_COLS}"
        ))
        .bind(c.id.0)
        .bind(self.org.0)
        .bind(&c.body)
        .bind(c.user_id.map(|u| u.0))
        .bind(c.parent.uuid())
        .bind(c.parent_id.map(|p| p.0))
        .bind(&c.quoted_text)
        .bind(c.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.into_comment()
    }

    async fn get(&mut self, id: CommentId) -> AppResult<Option<Comment>> {
        let row = sqlx::query_as::<_, CommentRow>(&format!(
            "SELECT {COMMENT_COLS} FROM comment WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(CommentRow::into_comment).transpose()
    }

    async fn list(&mut self, q: &CommentQuery) -> AppResult<Vec<Comment>> {
        let key = match q.order_by {
            OrderBy::CreatedAt => "created_at",
            OrderBy::UpdatedAt => "updated_at",
        };
        let dir = if q.descending { "DESC" } else { "ASC" };
        let cmp = if q.descending { "<" } else { ">" };
        let mut b: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {COMMENT_COLS} FROM comment WHERE archived_at IS NULL AND organization_id = "
        ));
        b.push_bind(self.org.0);
        b.push(format!(" AND {} = ", parent_col(q.parent.kind())));
        b.push_bind(q.parent.uuid());
        if let Some(c) = &q.cursor {
            b.push(format!(" AND ({key}, id) {cmp} ("));
            b.push_bind(c.key);
            b.push(", ");
            b.push_bind(c.id);
            b.push(")");
        }
        b.push(format!(" ORDER BY {key} {dir}, id {dir} LIMIT "));
        b.push_bind(q.fetch);
        let rows = b
            .build_query_as::<CommentRow>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        rows.into_iter().map(CommentRow::into_comment).collect()
    }

    async fn count_replies(&mut self, root: CommentId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM comment WHERE organization_id = $1 AND parent_id = $2 \
             AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(root.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn set_anchor(&mut self, id: CommentId, start: &str, end: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE comment SET anchor_start = $3, anchor_end = $4 \
             WHERE organization_id = $1 AND id = $2 AND parent_id IS NULL AND quoted_text IS NOT NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(start)
        .bind(end)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn inline_without_anchor(
        &mut self,
        document: DocumentId,
    ) -> AppResult<Vec<(CommentId, String)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, quoted_text FROM comment WHERE organization_id = $1 AND document_id = $2 \
             AND parent_id IS NULL AND quoted_text IS NOT NULL AND anchor_start IS NULL \
             AND archived_at IS NULL ORDER BY created_at, id LIMIT 500",
        )
        .bind(self.org.0)
        .bind(document.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(i, q)| (CommentId(i), q)).collect())
    }

    async fn update_body(
        &mut self,
        id: CommentId,
        body: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Comment> {
        let row = sqlx::query_as::<_, CommentRow>(&format!(
            "UPDATE comment SET body = $3, edited_at = $4, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND id = $2 RETURNING {COMMENT_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(body)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::not_found("Could not find referenced Comment."))?;
        row.into_comment()
    }

    async fn remove(
        &mut self,
        id: CommentId,
        placeholder: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let query = match placeholder {
            Some(text) => sqlx::query(
                "UPDATE comment SET body = $3, deleted_at = $4, updated_at = $4, version = version + 1 \
                 WHERE organization_id = $1 AND id = $2",
            )
            .bind(self.org.0)
            .bind(id.0)
            .bind(text.to_owned())
            .bind(now),
            None => sqlx::query(
                "UPDATE comment SET archived_at = $3, updated_at = $3, version = version + 1 \
                 WHERE organization_id = $1 AND id = $2",
            )
            .bind(self.org.0)
            .bind(id.0)
            .bind(now),
        };
        query.execute(&mut *self.tx).await.map_err(db_err)?;
        sqlx::query(
            "UPDATE reaction SET archived_at = $3, updated_at = $3 \
             WHERE organization_id = $1 AND comment_id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_resolution(
        &mut self,
        id: CommentId,
        resolution: Option<(UserId, Option<CommentId>)>,
        now: DateTime<Utc>,
    ) -> AppResult<Comment> {
        let (at, user, comment) = match resolution {
            Some((u, c)) => (Some(now), Some(u.0), c.map(|c| c.0)),
            None => (None, None, None),
        };
        let row = sqlx::query_as::<_, CommentRow>(&format!(
            "UPDATE comment SET resolved_at = $3, resolving_user_id = $4, resolving_comment_id = $5, \
             updated_at = $6, version = version + 1 \
             WHERE organization_id = $1 AND id = $2 RETURNING {COMMENT_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(at)
        .bind(user)
        .bind(comment)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::not_found("Could not find referenced Comment."))?;
        row.into_comment()
    }

    async fn mention_candidates(&mut self, names: &[String]) -> AppResult<Vec<MentionCandidate>> {
        if names.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
            "SELECT id, display_name, name, role FROM \"user\" \
             WHERE organization_id = $1 AND (display_name = ANY($2) OR lower(name) = ANY($2)) \
             AND active AND status = 'active'",
        )
        .bind(self.org.0)
        .bind(names)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|(id, display_name, name, role)| {
                Ok(MentionCandidate {
                    id: UserId(id),
                    display_name,
                    name,
                    role: role
                        .parse()
                        .map_err(|_| AppError::internal("invalid user role in database"))?,
                })
            })
            .collect()
    }

    async fn add_reaction(
        &mut self,
        target: ReactionTarget,
        user: UserId,
        emoji: &str,
        now: DateTime<Utc>,
    ) -> AppResult<(Reaction, bool)> {
        let (col, tid) = reaction_col(target);
        let new_id = ReactionId::new_v7();
        let inserted: Option<Uuid> = sqlx::query_scalar(&format!(
            "INSERT INTO reaction (id, organization_id, emoji, user_id, {col}, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT DO NOTHING RETURNING id"
        ))
        .bind(new_id.0)
        .bind(self.org.0)
        .bind(emoji)
        .bind(user.0)
        .bind(tid)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let (id, created) = match inserted {
            Some(id) => (id, true),
            None => {
                let id: Uuid = sqlx::query_scalar(&format!(
                    "SELECT id FROM reaction WHERE organization_id = $1 AND user_id = $2 \
                     AND emoji = $3 AND {col} = $4"
                ))
                .bind(self.org.0)
                .bind(user.0)
                .bind(emoji)
                .bind(tid)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(db_err)?;
                (id, false)
            }
        };
        let reaction = self
            .get_reaction(ReactionId(id))
            .await?
            .ok_or_else(|| AppError::internal("reaction vanished"))?;
        Ok((reaction, created))
    }

    async fn get_reaction(&mut self, id: ReactionId) -> AppResult<Option<Reaction>> {
        let row: Option<ReactionRow> = sqlx::query_as(
            "SELECT id, emoji, user_id, issue_id, comment_id, project_update_id, initiative_update_id, \
             created_at FROM reaction WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(ReactionRow::into_reaction).transpose()
    }

    async fn delete_reaction(&mut self, id: ReactionId) -> AppResult<()> {
        sqlx::query("DELETE FROM reaction WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn reactions_of_comments(&mut self, ids: &[CommentId]) -> AppResult<Vec<Reaction>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let raw: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        let rows: Vec<ReactionRow> = sqlx::query_as(
            "SELECT id, emoji, user_id, issue_id, comment_id, project_update_id, initiative_update_id, \
             created_at FROM reaction WHERE organization_id = $1 AND comment_id = ANY($2) \
             AND archived_at IS NULL ORDER BY created_at, id",
        )
        .bind(self.org.0)
        .bind(&raw)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ReactionRow::into_reaction).collect()
    }

    async fn subscribe(
        &mut self,
        issue: IssueId,
        user: UserId,
        thread: Option<CommentId>,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO issue_subscription (id, organization_id, issue_id, user_id, thread_comment_id, \
             reason, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (issue_id, user_id, thread_comment_id) DO NOTHING RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .bind(thread.map(|t| t.0))
        .bind(reason)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id.is_some())
    }
}

impl PgTx {
    async fn project_scope(&mut self, project: Uuid) -> AppResult<Option<ParentScope>> {
        let teams = self.project_team_ids(project).await?;
        Ok((!teams.is_empty()).then_some(ParentScope::Teams(teams)))
    }

    /// Projekt oder Initiative per UUID, Kennung (`P-ENG-7`, `I-4`), Slug oder Namen.
    async fn find_named(
        &mut self,
        reference: &str,
        table: &str,
        kind: &str,
    ) -> AppResult<Option<Uuid>> {
        let live = if table == "project" || table == "initiative" {
            "NOT trashed AND archived_at IS NULL"
        } else {
            "archived_at IS NULL"
        };
        if let Ok(id) = Uuid::parse_str(reference) {
            return sqlx::query_scalar(&format!(
                "SELECT id FROM {table} WHERE organization_id = $1 AND id = $2 AND {live}"
            ))
            .bind(self.org.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err);
        }
        if table == "project"
            && let Some((key, n)) = split_identifier(reference, "P-")
        {
            let sql = "SELECT p.id FROM project p JOIN team t ON t.organization_id = p.organization_id \
                       AND t.id = p.lead_team_id WHERE p.organization_id = $1 AND upper(t.key) = upper($2) \
                       AND p.number = $3 AND NOT p.trashed AND p.archived_at IS NULL";
            if let Some(id) = self.one_or_ambiguous(sql, &key, Some(n), kind).await? {
                return Ok(Some(id));
            }
        }
        if table == "initiative"
            && let Some((_, n)) = split_identifier(reference, "I-")
        {
            let id: Option<Uuid> = sqlx::query_scalar(&format!(
                "SELECT id FROM initiative WHERE organization_id = $1 AND number = $2 AND {live}"
            ))
            .bind(self.org.0)
            .bind(n)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            if id.is_some() {
                return Ok(id);
            }
        }
        if let Some(slug) = slug_of(reference) {
            let id: Option<Uuid> = sqlx::query_scalar(&format!(
                "SELECT id FROM {table} WHERE organization_id = $1 AND slug_id = $2 AND {live}"
            ))
            .bind(self.org.0)
            .bind(slug.to_ascii_lowercase())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            if id.is_some() {
                return Ok(id);
            }
        }
        let sql = format!(
            "SELECT id FROM {table} WHERE organization_id = $1 AND lower(name) = lower($2) AND {live}"
        );
        self.one_or_ambiguous(&sql, reference, None, kind).await
    }
}

fn reaction_col(target: ReactionTarget) -> (&'static str, Uuid) {
    match target {
        ReactionTarget::Comment(i) => ("comment_id", i.0),
        ReactionTarget::Issue(i) => ("issue_id", i.0),
        ReactionTarget::ProjectUpdate(i) => ("project_update_id", i.0),
        ReactionTarget::InitiativeUpdate(i) => ("initiative_update_id", i.0),
    }
}

#[derive(sqlx::FromRow)]
struct ReactionRow {
    id: Uuid,
    emoji: String,
    user_id: Uuid,
    issue_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    project_update_id: Option<Uuid>,
    initiative_update_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl ReactionRow {
    fn into_reaction(self) -> AppResult<Reaction> {
        let target = if let Some(i) = self.comment_id {
            ReactionTarget::Comment(CommentId(i))
        } else if let Some(i) = self.issue_id {
            ReactionTarget::Issue(IssueId(i))
        } else if let Some(i) = self.project_update_id {
            ReactionTarget::ProjectUpdate(ProjectUpdateId(i))
        } else if let Some(i) = self.initiative_update_id {
            ReactionTarget::InitiativeUpdate(InitiativeUpdateId(i))
        } else {
            return Err(AppError::internal("reaction without target"));
        };
        Ok(Reaction {
            id: ReactionId(self.id),
            emoji: self.emoji,
            user_id: UserId(self.user_id),
            target,
            created_at: self.created_at,
        })
    }
}
