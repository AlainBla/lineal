//! GraphQL-Teilobjekte des Bereichs `document` (WP10): Dokumente, Suche, Snapshot-Historie.
//! Regeln: docs/06 §1; Sichtbarkeit und Rechte prüft der Use-Case (nicht sichtbar → `not_found`).

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::document::{
    DocumentView, ListDocumentsInput, ParentRefs, SaveDocumentInput,
};
use lineal_domain::rules::patch::PatchOp;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `Document` (Inhalt als Markdown).
#[derive(SimpleObject)]
#[graphql(name = "Document")]
pub struct DocumentGql {
    pub id: String,
    pub slug_id: String,
    pub title: String,
    pub content: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub url: String,
    pub sort_order: f64,
    pub creator_id: Option<String>,
    pub updated_by_id: Option<String>,
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub initiative_id: Option<String>,
    pub cycle_id: Option<String>,
    pub team_id: Option<String>,
    /// Revision des Inhalts (für `expectedVersion`).
    pub content_version: i32,
    pub trashed: bool,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub warnings: Vec<String>,
}

impl From<DocumentView> for DocumentGql {
    fn from(v: DocumentView) -> Self {
        let d = v.doc;
        // Bei Cycle-Dokumenten ist `team_id` nur die Zugriffsspalte.
        let team_id = if d.cycle_id.is_some() {
            None
        } else {
            d.team_id
        };
        Self {
            id: d.id.to_string(),
            slug_id: d.slug_id,
            title: d.title,
            content: d.content,
            icon: d.icon,
            color: d.color,
            url: v.url,
            sort_order: d.sort_order,
            creator_id: d.creator_id.map(|u| u.to_string()),
            updated_by_id: d.updated_by_id.map(|u| u.to_string()),
            project_id: d.project_id.map(|u| u.to_string()),
            issue_id: d.issue_id.map(|u| u.to_string()),
            initiative_id: d.initiative_id.map(|u| u.to_string()),
            cycle_id: d.cycle_id.map(|u| u.to_string()),
            team_id: team_id.map(|u| u.to_string()),
            content_version: d.content_version,
            trashed: d.trashed,
            created_at: ts(d.created_at),
            updated_at: ts(d.updated_at),
            archived_at: d.archived_at.map(ts),
            warnings: v.warnings,
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "DocumentConnection")]
pub struct DocumentConnection {
    pub nodes: Vec<DocumentGql>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "DocumentPayload")]
pub struct DocumentPayload {
    pub success: bool,
    pub document: Option<DocumentGql>,
}

/// Ein Snapshot der Historie (`DocumentContentHistory`).
#[derive(SimpleObject)]
#[graphql(name = "DocumentContentHistory")]
pub struct HistoryGql {
    pub id: String,
    pub markdown: String,
    pub title: Option<String>,
    pub actor_ids: Vec<String>,
    pub kind: String,
    pub restored_from_id: Option<String>,
    pub content_snapshot_at: String,
}

#[derive(SimpleObject)]
#[graphql(name = "DocumentContentHistoryConnection")]
pub struct HistoryConnection {
    pub nodes: Vec<HistoryGql>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DocumentCreateInput")]
pub struct DocumentCreateInput {
    /// Pflicht, außer bei `templateId` (dann gilt der Template-Titel).
    pub title: Option<String>,
    pub content: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub initiative_id: Option<String>,
    pub cycle_id: Option<String>,
    /// Elternobjekt Team – oder bei `cycleId` die Auflösungshilfe für den Cycle.
    pub team_id: Option<String>,
    pub sort_order: Option<f64>,
    pub template_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DocumentUpdateInput")]
pub struct DocumentUpdateInput {
    pub title: Option<String>,
    pub content: Option<String>,
    /// Teiländerungen als JSON-Liste (`op`, …), schließt `content` aus.
    pub patch: Option<async_graphql::Json<serde_json::Value>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// Reparent: genau ein Elternparameter.
    pub project_id: Option<String>,
    pub issue_id: Option<String>,
    pub initiative_id: Option<String>,
    pub cycle_id: Option<String>,
    pub team_id: Option<String>,
    pub sort_order: Option<f64>,
    /// Optimistische Sperre auf die Inhalts-Revision.
    pub expected_version: Option<i32>,
    /// Automatisches Speichern des Editors: Inhaltsänderungen werden zu Snapshots gebündelt
    /// (docs/06 §1.9). Ohne Angabe erzeugt jede Inhaltsänderung einen Snapshot.
    pub autosave: Option<bool>,
}

fn cursor_of(c: &PageCursor) -> String {
    format!(
        "{}|{}",
        c.key.to_rfc3339_opts(SecondsFormat::Micros, true),
        c.id
    )
}

fn parse_cursor(s: &str) -> Result<PageCursor, AppError> {
    let bad = || AppError::invalid_request("invalid cursor");
    let (k, i) = s.split_once('|').ok_or_else(bad)?;
    Ok(PageCursor {
        key: DateTime::parse_from_rfc3339(k)
            .map_err(|_| bad())?
            .with_timezone(&Utc),
        id: i.parse().map_err(|_| bad())?,
    })
}

#[derive(Default)]
pub struct DocumentQuery;

#[Object]
impl DocumentQuery {
    /// Ein Dokument per UUID, `slugId` oder URL.
    async fn document(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<DocumentGql> {
        Ok(app(ctx)?
            .get_document(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Dokumente sichtbarer Elternobjekte (`trashed: true` = „Recently deleted“).
    #[allow(clippy::too_many_arguments)]
    async fn documents(
        &self,
        ctx: &Context<'_>,
        project_id: Option<String>,
        initiative_id: Option<String>,
        team_id: Option<String>,
        issue_id: Option<String>,
        cycle_id: Option<String>,
        creator_id: Option<String>,
        query: Option<String>,
        include_archived: Option<bool>,
        trashed: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<DocumentConnection> {
        let cursor = after
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(parse_cursor)
            .transpose()
            .map_err(gql_err)?;
        list(
            ctx,
            ListDocumentsInput {
                query,
                creator: creator_id,
                project: project_id,
                initiative: initiative_id,
                team: team_id,
                issue: issue_id,
                cycle: cycle_id,
                include_archived: include_archived.unwrap_or(false),
                trashed: trashed.unwrap_or(false),
                order_by: OrderBy::UpdatedAt,
                limit: i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 50, 250),
                    ctx,
                )?),
                cursor,
                ..Default::default()
            },
        )
        .await
    }

    /// Volltextsuche über Titel und Inhalt sichtbarer Dokumente.
    async fn search_documents(
        &self,
        ctx: &Context<'_>,
        term: String,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<DocumentConnection> {
        let cursor = after
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(parse_cursor)
            .transpose()
            .map_err(gql_err)?;
        list(
            ctx,
            ListDocumentsInput {
                query: Some(term),
                order_by: OrderBy::UpdatedAt,
                limit: i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 50, 250),
                    ctx,
                )?),
                cursor,
                ..Default::default()
            },
        )
        .await
    }

    /// Snapshot-Historie (neueste zuerst).
    async fn document_content_history(
        &self,
        ctx: &Context<'_>,
        document_id: String,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<HistoryConnection> {
        let before = after
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(|c| parse_cursor(c).map(|p| (p.key, p.id)))
            .transpose()
            .map_err(gql_err)?;
        let page = app(ctx)?
            .document_history(
                request_ctx(ctx)?,
                &document_id,
                before,
                i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 50, 250),
                    ctx,
                )?),
            )
            .await
            .map_err(gql_err)?;
        Ok(HistoryConnection {
            has_next_page: page.next.is_some(),
            end_cursor: page
                .next
                .map(|(k, i)| cursor_of(&PageCursor { key: k, id: i })),
            nodes: page
                .items
                .into_iter()
                .map(|h| HistoryGql {
                    id: h.id.to_string(),
                    markdown: h.markdown,
                    title: h.title,
                    actor_ids: h.actor_ids.iter().map(|u| u.to_string()).collect(),
                    kind: h.kind.as_str().to_owned(),
                    restored_from_id: h.restored_from_id.map(|u| u.to_string()),
                    content_snapshot_at: ts(h.content_snapshot_at),
                })
                .collect(),
        })
    }
}

async fn list(
    ctx: &Context<'_>,
    input: ListDocumentsInput,
) -> async_graphql::Result<DocumentConnection> {
    let page = app(ctx)?
        .list_documents(request_ctx(ctx)?, input)
        .await
        .map_err(gql_err)?;
    Ok(DocumentConnection {
        has_next_page: page.next.is_some(),
        end_cursor: page.next.as_ref().map(cursor_of),
        nodes: page.items.into_iter().map(Into::into).collect(),
    })
}

fn payload(v: DocumentView) -> DocumentPayload {
    DocumentPayload {
        success: true,
        document: Some(v.into()),
    }
}

#[derive(Default)]
pub struct DocumentMutation;

#[Object]
impl DocumentMutation {
    async fn document_create(
        &self,
        ctx: &Context<'_>,
        input: DocumentCreateInput,
    ) -> async_graphql::Result<DocumentPayload> {
        let v = app(ctx)?
            .save_document(
                request_ctx(ctx)?,
                SaveDocumentInput {
                    title: input.title,
                    content: input.content,
                    icon: input.icon,
                    color: input.color,
                    parent: ParentRefs {
                        project: input.project_id,
                        issue: input.issue_id,
                        initiative: input.initiative_id,
                        cycle: input.cycle_id,
                        team: input.team_id,
                    },
                    sort_order: input.sort_order,
                    template: input.template_id,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn document_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: DocumentUpdateInput,
    ) -> async_graphql::Result<DocumentPayload> {
        let patch: Option<Vec<PatchOp>> = input
            .patch
            .map(|p| serde_json::from_value(p.0))
            .transpose()
            .map_err(|e| gql_err(AppError::invalid_request(format!("invalid patch: {e}"))))?;
        let autosave = input.autosave.unwrap_or(false);
        let a = app(ctx)?;
        let save_input = SaveDocumentInput {
            id: Some(id),
            title: input.title,
            content: input.content,
            patch,
            icon: input.icon,
            color: input.color,
            parent: ParentRefs {
                project: input.project_id,
                issue: input.issue_id,
                initiative: input.initiative_id,
                cycle: input.cycle_id,
                team: input.team_id,
            },
            sort_order: input.sort_order,
            expected_version: input.expected_version,
            template: None,
        };
        let v = if autosave {
            a.save_document_autosave(request_ctx(ctx)?, save_input)
                .await
        } else {
            a.save_document(request_ctx(ctx)?, save_input).await
        }
        .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Papierkorb (30 Tage wiederherstellbar).
    async fn document_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<DocumentPayload> {
        Ok(payload(
            app(ctx)?
                .delete_document(request_ctx(ctx)?, &id)
                .await
                .map_err(gql_err)?,
        ))
    }

    async fn document_unarchive(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<DocumentPayload> {
        Ok(payload(
            app(ctx)?
                .restore_document(request_ctx(ctx)?, &id)
                .await
                .map_err(gql_err)?,
        ))
    }

    /// Stellt den Inhalt eines Snapshots als neue Version wieder her.
    async fn document_content_restore(
        &self,
        ctx: &Context<'_>,
        document_id: String,
        history_id: String,
    ) -> async_graphql::Result<DocumentPayload> {
        Ok(payload(
            app(ctx)?
                .restore_document_version(request_ctx(ctx)?, &document_id, &history_id)
                .await
                .map_err(gql_err)?,
        ))
    }
}
