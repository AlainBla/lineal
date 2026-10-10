//! GraphQL-Teilobjekte des Bereichs `search` (WP11, docs/05 §5, docs/09 §1.4): `searchIssues`,
//! `searchProjects`, `searchDocuments`. Relevanzsortierung, höchstens 500 Treffer, Ausschnitte
//! (`highlights`) nur aus sichtbaren Objekten; Kommentare nur mit `includeComments`.

use super::issue::IssueGql;
use crate::context::{app, request_ctx};
use crate::error::GqlResultExt;
use crate::pagination::PaginationOrderBy;
use async_graphql::types::Json;
use async_graphql::{Context, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::view::{SearchHitRow, SearchKind, SearchOrder};
use lineal_app::usecases::view::{SearchInput, SearchPage};
use serde_json::Value;

/// Trefferausschnitt (`<mark>` um Fundstellen, übriger Text HTML-escaped).
#[derive(SimpleObject)]
pub struct SearchHighlight {
    pub field: String,
    pub snippet: String,
}

fn highlights(h: &SearchHitRow) -> Vec<SearchHighlight> {
    h.highlights
        .iter()
        .map(|(field, snippet)| SearchHighlight {
            field: field.clone(),
            snippet: snippet.clone(),
        })
        .collect()
}

#[derive(SimpleObject)]
pub struct IssueSearchResult {
    pub issue: IssueGql,
    pub highlights: Vec<SearchHighlight>,
}

#[derive(SimpleObject)]
pub struct IssueSearchConnection {
    pub nodes: Vec<IssueSearchResult>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

/// Treffer eines Projekts, einer Initiative oder eines Dokuments.
#[derive(SimpleObject)]
pub struct EntitySearchResult {
    pub id: String,
    /// `Project`, `Initiative`, `Document`.
    pub kind: String,
    pub identifier: Option<String>,
    pub title: String,
    pub team_key: Option<String>,
    pub status: Option<String>,
    pub archived: bool,
    pub updated_at: String,
    pub highlights: Vec<SearchHighlight>,
}

#[derive(SimpleObject)]
pub struct EntitySearchConnection {
    pub nodes: Vec<EntitySearchResult>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

fn offset_of(after: Option<&str>) -> Result<i64, AppError> {
    match after {
        None => Ok(0),
        Some(c) => c
            .strip_prefix("o:")
            .and_then(|n| n.parse::<i64>().ok())
            .filter(|n| (0..500).contains(n))
            .ok_or_else(|| AppError::invalid_request("invalid cursor")),
    }
}

fn entity_page(page: SearchPage) -> EntitySearchConnection {
    EntitySearchConnection {
        has_next_page: page.next_offset.is_some(),
        end_cursor: page.next_offset.map(|o| format!("o:{o}")),
        nodes: page
            .hits
            .iter()
            .map(|h| EntitySearchResult {
                id: h.id.to_string(),
                kind: match h.kind {
                    SearchKind::Issue => "Issue",
                    SearchKind::Project => "Project",
                    SearchKind::Initiative => "Initiative",
                    SearchKind::Document => "Document",
                }
                .into(),
                identifier: h.identifier.clone(),
                title: h.title.clone(),
                team_key: h.team_key.clone(),
                status: h.status.clone(),
                archived: h.archived,
                updated_at: h.updated_at.to_rfc3339(),
                highlights: highlights(h),
            })
            .collect(),
    }
}

#[derive(Default)]
pub struct SearchQuery;

#[Object]
impl SearchQuery {
    /// Volltextsuche über Issues (inkl. früherer Kennungen), mit Zusatzfilter (`IssueFilter`).
    #[allow(clippy::too_many_arguments)]
    async fn search_issues(
        &self,
        ctx: &Context<'_>,
        term: String,
        filter: Option<Json<Value>>,
        team_id: Option<String>,
        include_comments: Option<bool>,
        include_archived: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
        order_by: Option<PaginationOrderBy>,
        relevance: Option<bool>,
    ) -> async_graphql::Result<IssueSearchConnection> {
        let offset = offset_of(after.as_deref()).gql(ctx)?;
        let order = match (relevance.unwrap_or(true), order_by) {
            (true, _) => SearchOrder::Relevance,
            (false, Some(PaginationOrderBy::CreatedAt)) => SearchOrder::CreatedAt,
            (false, _) => SearchOrder::UpdatedAt,
        };
        let page = app(ctx)?
            .search(
                request_ctx(ctx)?,
                SearchInput {
                    term,
                    kinds: vec![SearchKind::Issue],
                    filter: filter.map(|f| f.0),
                    team: team_id,
                    include_comments: include_comments.unwrap_or(false),
                    include_archived: include_archived.unwrap_or(true),
                    order,
                    offset,
                    limit: i64::from(first.unwrap_or(50)),
                },
            )
            .await
            .gql(ctx)?;
        let mut issues = page.issues;
        let nodes = page
            .hits
            .iter()
            .filter_map(|h| {
                issues.remove(&h.id).map(|v| IssueSearchResult {
                    issue: v.into(),
                    highlights: highlights(h),
                })
            })
            .collect();
        Ok(IssueSearchConnection {
            nodes,
            has_next_page: page.next_offset.is_some(),
            end_cursor: page.next_offset.map(|o| format!("o:{o}")),
        })
    }

    async fn search_projects(
        &self,
        ctx: &Context<'_>,
        term: String,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<EntitySearchConnection> {
        let offset = offset_of(after.as_deref()).gql(ctx)?;
        let page = app(ctx)?
            .search(
                request_ctx(ctx)?,
                SearchInput {
                    term,
                    kinds: vec![SearchKind::Project],
                    offset,
                    limit: i64::from(first.unwrap_or(50)),
                    ..Default::default()
                },
            )
            .await
            .gql(ctx)?;
        Ok(entity_page(page))
    }

    async fn search_documents(
        &self,
        ctx: &Context<'_>,
        term: String,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<EntitySearchConnection> {
        let offset = offset_of(after.as_deref()).gql(ctx)?;
        let page = app(ctx)?
            .search(
                request_ctx(ctx)?,
                SearchInput {
                    term,
                    kinds: vec![SearchKind::Document],
                    offset,
                    limit: i64::from(first.unwrap_or(50)),
                    ..Default::default()
                },
            )
            .await
            .gql(ctx)?;
        Ok(entity_page(page))
    }

    /// Globale Suche („All“-Tab): Issues, Projekte, Initiativen und Dokumente gemischt nach
    /// Relevanz; `types` schränkt ein (`Issue`, `Project`, `Initiative`, `Document`).
    async fn search(
        &self,
        ctx: &Context<'_>,
        term: String,
        types: Option<Vec<String>>,
        include_comments: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<EntitySearchConnection> {
        let offset = offset_of(after.as_deref()).gql(ctx)?;
        let kinds = types
            .unwrap_or_default()
            .iter()
            .map(|t| match t.as_str() {
                "Issue" => Ok(SearchKind::Issue),
                "Project" => Ok(SearchKind::Project),
                "Initiative" => Ok(SearchKind::Initiative),
                "Document" => Ok(SearchKind::Document),
                other => Err(AppError::invalid_request(format!("unknown search type `{other}`"))),
            })
            .collect::<Result<Vec<_>, _>>()
            .gql(ctx)?;
        let page = app(ctx)?
            .search(
                request_ctx(ctx)?,
                SearchInput {
                    term,
                    kinds,
                    include_comments: include_comments.unwrap_or(false),
                    offset,
                    limit: i64::from(first.unwrap_or(50)),
                    ..Default::default()
                },
            )
            .await
            .gql(ctx)?;
        Ok(entity_page(page))
    }
}

/// Die Suche kennt keine Mutationen; der Typ bleibt für die `MergedObject`-Struktur.
#[derive(Default)]
pub struct SearchMutation;

#[Object]
impl SearchMutation {
    /// Platzhalter des Scaffolds (der Mutation-Root darf nicht leer sein).
    #[graphql(name = "_searchMutationReady")]
    async fn search_mutation_ready(&self) -> bool {
        true
    }
}
