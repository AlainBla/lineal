//! GraphQL-Teilobjekte des Bereichs `comment` (WP09): Kommentare, Reaktionen, Anhänge, Uploads.
//!
//! Typnamen (`Comment`, `Reaction`, `Attachment`, `UploadPayload` …) folgen docs/09; die
//! Lösch-Payloads heißen `CommentDeletePayload`/`ReactionDeletePayload`/`AttachmentDeletePayload`,
//! damit sie nicht mit dem `DeletePayload` anderer Bereiche kollidieren.

use crate::context::{app, request_ctx};
use crate::graphql::org::gql_err;
use crate::pagination::{Cursor, MAX_PAGE_SIZE, PageInfo, PaginationOrderBy, page_request};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::comment::{ParentKind, ReactionTarget};
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::attachment::{AttachmentView, LinkAttachmentInput, PrepareUploadInput};
use lineal_app::usecases::comment::{
    CommentView, CreateCommentInput, ListCommentsInput, ParentSpec, ReactionTargetSpec,
};

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn sync_id(n: i64) -> f64 {
    n as f64
}

/// Autor eines Kommentars.
#[derive(SimpleObject)]
#[graphql(name = "CommentUser")]
pub struct CommentUserGql {
    pub id: String,
    pub name: String,
    pub display_name: String,
}

/// Aggregierte Reaktionen (`reactionData`).
#[derive(SimpleObject)]
#[graphql(name = "ReactionData")]
pub struct ReactionDataGql {
    pub emoji: String,
    pub count: i32,
    pub user_ids: Vec<String>,
}

/// `Comment` (docs/03 §6).
#[derive(SimpleObject)]
#[graphql(name = "Comment")]
pub struct CommentGql {
    pub id: String,
    pub body: String,
    pub user_id: Option<String>,
    pub user: Option<CommentUserGql>,
    pub issue_id: Option<String>,
    pub project_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub project_update_id: Option<String>,
    pub initiative_id: Option<String>,
    pub initiative_update_id: Option<String>,
    pub document_id: Option<String>,
    pub parent_id: Option<String>,
    pub quoted_text: Option<String>,
    pub edited_at: Option<String>,
    pub resolved_at: Option<String>,
    pub resolving_user_id: Option<String>,
    pub resolving_comment_id: Option<String>,
    pub reaction_data: Vec<ReactionDataGql>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<CommentView> for CommentGql {
    fn from(v: CommentView) -> Self {
        let c = v.comment;
        let parent = c.parent;
        let pick = |kind: ParentKind| (parent.kind() == kind).then(|| parent.uuid().to_string());
        Self {
            id: c.id.to_string(),
            body: c.body,
            user_id: c.user_id.map(|u| u.to_string()),
            user: v.author.map(|a| CommentUserGql {
                id: a.id.to_string(),
                name: a.name,
                display_name: a.display_name,
            }),
            issue_id: pick(ParentKind::Issue),
            project_id: pick(ParentKind::Project),
            project_milestone_id: pick(ParentKind::Milestone),
            project_update_id: pick(ParentKind::ProjectUpdate),
            initiative_id: pick(ParentKind::Initiative),
            initiative_update_id: pick(ParentKind::InitiativeUpdate),
            document_id: pick(ParentKind::Document),
            parent_id: c.parent_id.map(|p| p.to_string()),
            quoted_text: c.quoted_text,
            edited_at: c.edited_at.map(ts),
            resolved_at: c.resolved_at.map(ts),
            resolving_user_id: c.resolving_user_id.map(|u| u.to_string()),
            resolving_comment_id: c.resolving_comment_id.map(|u| u.to_string()),
            reaction_data: v
                .reactions
                .into_iter()
                .map(|g| ReactionDataGql {
                    emoji: g.emoji,
                    count: i32::try_from(g.count).unwrap_or(i32::MAX),
                    user_ids: g.user_ids.iter().map(|u| u.to_string()).collect(),
                })
                .collect(),
            created_at: ts(c.created_at),
            updated_at: ts(c.updated_at),
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "CommentConnection")]
pub struct CommentConnection {
    pub nodes: Vec<CommentGql>,
    pub page_info: PageInfo,
}

#[derive(InputObject)]
#[graphql(name = "CommentCreateInput")]
pub struct CommentCreateInput {
    pub body: String,
    pub issue_id: Option<String>,
    pub project_id: Option<String>,
    pub initiative_id: Option<String>,
    pub document_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub project_update_id: Option<String>,
    pub initiative_update_id: Option<String>,
    /// Antwort auf diesen Kommentar (Thread-Wurzel wird normalisiert).
    pub parent_id: Option<String>,
    /// Inline-Kommentar: zitierte Textstelle der Beschreibung.
    pub quoted_text: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "CommentUpdateInput")]
pub struct CommentUpdateInput {
    pub body: String,
}

#[derive(SimpleObject)]
#[graphql(name = "CommentPayload")]
pub struct CommentPayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub comment: Option<CommentGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "CommentDeletePayload")]
pub struct CommentDeletePayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub entity_id: String,
}

/// `Reaction`.
#[derive(SimpleObject)]
#[graphql(name = "Reaction")]
pub struct ReactionGql {
    pub id: String,
    pub emoji: String,
    pub user_id: String,
    pub comment_id: Option<String>,
    pub issue_id: Option<String>,
    pub project_update_id: Option<String>,
    pub initiative_update_id: Option<String>,
    pub created_at: String,
}

#[derive(InputObject)]
#[graphql(name = "ReactionCreateInput")]
pub struct ReactionCreateInput {
    pub emoji: String,
    pub comment_id: Option<String>,
    pub issue_id: Option<String>,
    pub project_update_id: Option<String>,
    pub initiative_update_id: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "ReactionPayload")]
pub struct ReactionPayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub reaction: Option<ReactionGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "ReactionDeletePayload")]
pub struct ReactionDeletePayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub entity_id: String,
}

/// `Attachment` (Link oder hochgeladene Datei; `url` ist bei Uploads kurzlebig signiert).
#[derive(SimpleObject)]
#[graphql(name = "Attachment")]
pub struct AttachmentGql {
    pub id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub source_type: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<f64>,
    pub creator_id: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AttachmentView> for AttachmentGql {
    fn from(v: AttachmentView) -> Self {
        let a = v.attachment;
        Self {
            id: a.id.to_string(),
            issue_id: a.issue_id.to_string(),
            issue_identifier: v.issue_identifier,
            url: v.url,
            title: a.title,
            subtitle: a.subtitle,
            source_type: a.source_type,
            content_type: a.content_type,
            size: a.size.map(|s| s as f64),
            creator_id: a.creator_id.map(|u| u.to_string()),
            expires_at: v.expires_at.map(ts),
            created_at: ts(a.created_at),
            updated_at: ts(a.updated_at),
        }
    }
}

#[derive(InputObject)]
#[graphql(name = "AttachmentCreateInput")]
pub struct AttachmentCreateInput {
    pub issue_id: String,
    /// Link-URL oder die `assetUrl` eines abgeschlossenen Uploads (`fileUpload`).
    pub url: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "AttachmentPayload")]
pub struct AttachmentPayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub attachment: Option<AttachmentGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "AttachmentDeletePayload")]
pub struct AttachmentDeletePayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub entity_id: String,
}

#[derive(SimpleObject)]
#[graphql(name = "UploadFileHeader")]
pub struct UploadFileHeader {
    pub key: String,
    pub value: String,
}

#[derive(SimpleObject)]
#[graphql(name = "UploadFile")]
pub struct UploadFile {
    pub upload_url: String,
    pub headers: Vec<UploadFileHeader>,
    pub asset_url: String,
    pub filename: String,
    pub content_type: String,
    pub size: i32,
}

#[derive(SimpleObject)]
#[graphql(name = "UploadPayload")]
pub struct UploadPayload {
    pub success: bool,
    pub last_sync_id: f64,
    pub upload_file: Option<UploadFile>,
}

fn invalid(msg: &str) -> async_graphql::Error {
    gql_err(AppError::invalid_request(msg))
}

fn cursor_to_page(c: &Cursor) -> async_graphql::Result<PageCursor> {
    let key = DateTime::parse_from_rfc3339(&c.sort_key)
        .map_err(|_| invalid("Invalid cursor."))?
        .with_timezone(&Utc);
    Ok(PageCursor { key, id: c.id })
}

fn page_to_cursor(c: &PageCursor) -> Cursor {
    Cursor {
        sort_key: c.key.to_rfc3339_opts(SecondsFormat::Micros, true),
        id: c.id,
    }
}

#[derive(Default)]
pub struct CommentQuery;

#[Object]
impl CommentQuery {
    /// Ein Kommentar (Sichtbarkeit über das Elternobjekt).
    async fn comment(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<CommentGql> {
        let view = app(ctx)?
            .get_comment(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(view.into())
    }

    /// Kommentare eines Elternobjekts (genau eines der `*Id`-Argumente), neueste zuerst.
    #[allow(clippy::too_many_arguments)]
    async fn comments(
        &self,
        ctx: &Context<'_>,
        issue_id: Option<String>,
        project_id: Option<String>,
        initiative_id: Option<String>,
        document_id: Option<String>,
        project_milestone_id: Option<String>,
        project_update_id: Option<String>,
        initiative_update_id: Option<String>,
        first: Option<i32>,
        after: Option<String>,
        order_by: Option<PaginationOrderBy>,
    ) -> async_graphql::Result<CommentConnection> {
        let mut parents: Vec<ParentSpec> = Vec::new();
        parents.extend(issue_id.map(ParentSpec::Issue));
        parents.extend(project_id.map(ParentSpec::Project));
        parents.extend(initiative_id.map(ParentSpec::Initiative));
        parents.extend(document_id.map(ParentSpec::Document));
        parents.extend(project_milestone_id.map(ParentSpec::Milestone));
        parents.extend(project_update_id.map(|id| ParentSpec::StatusUpdate {
            id,
            kind: Some(ParentKind::ProjectUpdate),
        }));
        parents.extend(initiative_update_id.map(|id| ParentSpec::StatusUpdate {
            id,
            kind: Some(ParentKind::InitiativeUpdate),
        }));
        if parents.len() != 1 {
            return Err(invalid(
                "Provide exactly one of issueId, projectId, initiativeId, documentId, projectMilestoneId, projectUpdateId, or initiativeUpdateId",
            ));
        }
        let order = order_by.unwrap_or_default();
        let req = page_request(first, after.as_deref(), None, None, order, MAX_PAGE_SIZE)
            .map_err(gql_err)?;
        let cursor = req.after.as_ref().map(cursor_to_page).transpose()?;
        let page = app(ctx)?
            .list_comments(
                request_ctx(ctx)?,
                ListCommentsInput {
                    parent: parents.remove(0),
                    order_by: match order {
                        PaginationOrderBy::CreatedAt => OrderBy::CreatedAt,
                        PaginationOrderBy::UpdatedAt => OrderBy::UpdatedAt,
                    },
                    cursor,
                    limit: i64::from(req.limit),
                },
            )
            .await
            .map_err(gql_err)?;
        let first_cursor = page.items.first().map(|v| {
            page_to_cursor(&PageCursor {
                key: match order {
                    PaginationOrderBy::CreatedAt => v.comment.created_at,
                    PaginationOrderBy::UpdatedAt => v.comment.updated_at,
                },
                id: v.comment.id.0,
            })
            .encode(order)
        });
        let end = page.next.as_ref().map(|c| page_to_cursor(c).encode(order));
        Ok(CommentConnection {
            page_info: PageInfo {
                has_next_page: end.is_some(),
                has_previous_page: false,
                start_cursor: first_cursor,
                end_cursor: end,
            },
            nodes: page.items.into_iter().map(Into::into).collect(),
        })
    }

    /// Ein Anhang (Uploads mit frisch signierter Download-URL).
    async fn attachment(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<AttachmentGql> {
        let v = app(ctx)?
            .get_attachment(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(v.into())
    }
}

#[derive(Default)]
pub struct CommentMutation;

fn parent_spec(input: &CommentCreateInput) -> async_graphql::Result<Option<ParentSpec>> {
    let mut parents: Vec<ParentSpec> = Vec::new();
    parents.extend(input.issue_id.clone().map(ParentSpec::Issue));
    parents.extend(input.project_id.clone().map(ParentSpec::Project));
    parents.extend(input.initiative_id.clone().map(ParentSpec::Initiative));
    parents.extend(input.document_id.clone().map(ParentSpec::Document));
    parents.extend(
        input
            .project_milestone_id
            .clone()
            .map(ParentSpec::Milestone),
    );
    parents.extend(
        input
            .project_update_id
            .clone()
            .map(|id| ParentSpec::StatusUpdate {
                id,
                kind: Some(ParentKind::ProjectUpdate),
            }),
    );
    parents.extend(
        input
            .initiative_update_id
            .clone()
            .map(|id| ParentSpec::StatusUpdate {
                id,
                kind: Some(ParentKind::InitiativeUpdate),
            }),
    );
    if parents.len() > 1 {
        return Err(invalid("Provide exactly one parent object"));
    }
    Ok(parents.pop())
}

fn target_spec(input: &ReactionCreateInput) -> async_graphql::Result<ReactionTargetSpec> {
    let mut targets: Vec<ReactionTargetSpec> = Vec::new();
    targets.extend(input.comment_id.clone().map(ReactionTargetSpec::Comment));
    targets.extend(input.issue_id.clone().map(ReactionTargetSpec::Issue));
    targets.extend(
        input
            .project_update_id
            .clone()
            .map(ReactionTargetSpec::ProjectUpdate),
    );
    targets.extend(
        input
            .initiative_update_id
            .clone()
            .map(ReactionTargetSpec::InitiativeUpdate),
    );
    if targets.len() != 1 {
        return Err(invalid(
            "Provide exactly one of commentId, issueId, projectUpdateId, or initiativeUpdateId",
        ));
    }
    Ok(targets.remove(0))
}

#[Object]
impl CommentMutation {
    /// Neuer Thread, Antwort (`parentId`) oder Inline-Kommentar (`quotedText`).
    async fn comment_create(
        &self,
        ctx: &Context<'_>,
        input: CommentCreateInput,
    ) -> async_graphql::Result<CommentPayload> {
        let m = app(ctx)?
            .create_comment(
                request_ctx(ctx)?,
                CreateCommentInput {
                    parent: parent_spec(&input)?,
                    reply_to: input.parent_id.clone(),
                    body: input.body.clone(),
                    quoted_text: input.quoted_text.clone(),
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(CommentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            comment: Some(m.view.into()),
        })
    }

    /// Bearbeitet den Body (nur der Autor).
    async fn comment_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: CommentUpdateInput,
    ) -> async_graphql::Result<CommentPayload> {
        let m = app(ctx)?
            .update_comment(request_ctx(ctx)?, &id, &input.body)
            .await
            .map_err(gql_err)?;
        Ok(CommentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            comment: Some(m.view.into()),
        })
    }

    /// Löscht einen Kommentar (Autor oder Admin; Inline-Wurzeln nicht).
    async fn comment_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CommentDeletePayload> {
        let d = app(ctx)?
            .delete_comment(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(CommentDeletePayload {
            success: true,
            last_sync_id: sync_id(d.last_sync_id),
            entity_id: d.id.to_string(),
        })
    }

    /// Löst den Thread auf (optional mit hervorgehobener Antwort).
    async fn comment_resolve(
        &self,
        ctx: &Context<'_>,
        id: String,
        resolving_comment_id: Option<String>,
    ) -> async_graphql::Result<CommentPayload> {
        let m = app(ctx)?
            .resolve_comment(request_ctx(ctx)?, &id, resolving_comment_id.as_deref())
            .await
            .map_err(gql_err)?;
        Ok(CommentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            comment: Some(m.view.into()),
        })
    }

    /// Öffnet einen aufgelösten Thread wieder.
    async fn comment_unresolve(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CommentPayload> {
        let m = app(ctx)?
            .unresolve_comment(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(CommentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            comment: Some(m.view.into()),
        })
    }

    /// Reaktion hinzufügen (idempotent pro User, Ziel und Emoji).
    async fn reaction_create(
        &self,
        ctx: &Context<'_>,
        input: ReactionCreateInput,
    ) -> async_graphql::Result<ReactionPayload> {
        let m = app(ctx)?
            .add_reaction(request_ctx(ctx)?, target_spec(&input)?, &input.emoji)
            .await
            .map_err(gql_err)?;
        let r = m.reaction;
        let (mut comment_id, mut issue_id, mut pu, mut iu) = (None, None, None, None);
        match r.target {
            ReactionTarget::Comment(i) => comment_id = Some(i.to_string()),
            ReactionTarget::Issue(i) => issue_id = Some(i.to_string()),
            ReactionTarget::ProjectUpdate(i) => pu = Some(i.to_string()),
            ReactionTarget::InitiativeUpdate(i) => iu = Some(i.to_string()),
        }
        Ok(ReactionPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            reaction: Some(ReactionGql {
                id: r.id.to_string(),
                emoji: r.emoji,
                user_id: r.user_id.to_string(),
                comment_id,
                issue_id,
                project_update_id: pu,
                initiative_update_id: iu,
                created_at: ts(r.created_at),
            }),
        })
    }

    /// Entfernt die eigene Reaktion.
    async fn reaction_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<ReactionDeletePayload> {
        let d = app(ctx)?
            .remove_reaction(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(ReactionDeletePayload {
            success: true,
            last_sync_id: sync_id(d.last_sync_id),
            entity_id: d.id.to_string(),
        })
    }

    /// Hängt einen Link an oder finalisiert einen Upload (`url` = `assetUrl` aus `fileUpload`).
    async fn attachment_create(
        &self,
        ctx: &Context<'_>,
        input: AttachmentCreateInput,
    ) -> async_graphql::Result<AttachmentPayload> {
        let app = app(ctx)?;
        let rctx = request_ctx(ctx)?;
        let m = if input.url.starts_with(&app.asset_prefix()) {
            app.create_attachment_from_upload(
                rctx,
                &input.issue_id,
                &input.url,
                input.title.as_deref(),
                input.subtitle.as_deref(),
            )
            .await
        } else {
            app.link_attachment(
                rctx,
                LinkAttachmentInput {
                    issue: input.issue_id.clone(),
                    url: input.url.clone(),
                    title: input.title.clone(),
                    subtitle: input.subtitle.clone(),
                    metadata: None,
                },
            )
            .await
        }
        .map_err(gql_err)?;
        Ok(AttachmentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            attachment: Some(m.view.into()),
        })
    }

    /// Hängt einen Link an (Dedupe pro Issue und URL).
    #[graphql(name = "attachmentLinkURL")]
    async fn attachment_link_url(
        &self,
        ctx: &Context<'_>,
        issue_id: String,
        url: String,
        title: Option<String>,
    ) -> async_graphql::Result<AttachmentPayload> {
        let m = app(ctx)?
            .link_attachment(
                request_ctx(ctx)?,
                LinkAttachmentInput {
                    issue: issue_id,
                    url,
                    title,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(AttachmentPayload {
            success: true,
            last_sync_id: sync_id(m.last_sync_id),
            attachment: Some(m.view.into()),
        })
    }

    /// Entfernt einen Anhang.
    async fn attachment_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<AttachmentDeletePayload> {
        let d = app(ctx)?
            .delete_attachment(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(AttachmentDeletePayload {
            success: true,
            last_sync_id: sync_id(d.last_sync_id),
            entity_id: d.id.to_string(),
        })
    }

    /// Bereitet einen Direkt-Upload vor (ohne Ziel-Issue, z. B. für Bilder in Markdown).
    async fn file_upload(
        &self,
        ctx: &Context<'_>,
        filename: String,
        content_type: String,
        size: i32,
    ) -> async_graphql::Result<UploadPayload> {
        let p = app(ctx)?
            .prepare_attachment_upload(
                request_ctx(ctx)?,
                PrepareUploadInput {
                    issue: None,
                    filename: filename.clone(),
                    content_type: content_type.clone(),
                    size: i64::from(size),
                    title: None,
                    subtitle: None,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(UploadPayload {
            success: true,
            last_sync_id: 0.0,
            upload_file: Some(UploadFile {
                upload_url: p.url,
                headers: p
                    .headers
                    .into_iter()
                    .map(|(key, value)| UploadFileHeader { key, value })
                    .collect(),
                asset_url: p.asset_url,
                filename,
                content_type,
                size,
            }),
        })
    }
}

#[cfg(test)]
#[path = "comment_tests.rs"]
mod tests;
