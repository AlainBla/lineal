//! GraphQL-Schema: Root-Typen sind `MergedObject`s aus Teilobjekten pro Fachbereich.
//! Gemeinsame Datei: nur S1 / Orchestrator.

pub mod account;
pub mod api_key;
pub mod audit;
pub mod comment;
pub mod cycle;
pub mod document;
pub mod import;
pub mod initiative;
pub mod issue;
pub mod label;
pub mod notification;
pub mod org;
pub mod overview;
pub mod project;
pub mod search;
pub mod team;
pub mod template;
pub mod triage;
pub mod view;
pub mod webhook;

use async_graphql::{EmptySubscription, MergedObject, Schema};
use lineal_app::App;
use std::sync::Arc;

use account::{AccountMutation, AccountQuery};
use api_key::{ApiKeyMutation, ApiKeyQuery};
use audit::AuditQuery;
use comment::{CommentMutation, CommentQuery};
use cycle::{CycleMutation, CycleQuery};
use document::{DocumentMutation, DocumentQuery};
use import::{ImportMutation, ImportQuery};
use initiative::{InitiativeMutation, InitiativeQuery};
use issue::{IssueMutation, IssueQuery};
use label::{LabelMutation, LabelQuery};
use notification::{NotificationMutation, NotificationQuery};
use org::{OrgMutation, OrgQuery};
use overview::{OverviewMutation, OverviewQuery};
use project::{ProjectMutation, ProjectQuery};
use search::{SearchMutation, SearchQuery};
use team::{TeamMutation, TeamQuery};
use template::{TemplateMutation, TemplateQuery};
use triage::{TriageMutation, TriageQuery};
use view::{ViewMutation, ViewQuery};
use webhook::{WebhookMutation, WebhookQuery};

#[derive(MergedObject, Default)]
#[graphql(name = "Query")]
pub struct QueryRoot(
    OrgQuery,
    TeamQuery,
    IssueQuery,
    LabelQuery,
    CommentQuery,
    CycleQuery,
    ProjectQuery,
    InitiativeQuery,
    ViewQuery,
    SearchQuery,
    NotificationQuery,
    DocumentQuery,
    TemplateQuery,
    TriageQuery,
    OverviewQuery,
    WebhookQuery,
    ImportQuery,
    ApiKeyQuery,
    AuditQuery,
    AccountQuery,
);

#[derive(MergedObject, Default)]
#[graphql(name = "Mutation")]
pub struct MutationRoot(
    OrgMutation,
    TeamMutation,
    IssueMutation,
    LabelMutation,
    CommentMutation,
    CycleMutation,
    ProjectMutation,
    InitiativeMutation,
    ViewMutation,
    SearchMutation,
    NotificationMutation,
    DocumentMutation,
    TemplateMutation,
    TriageMutation,
    OverviewMutation,
    WebhookMutation,
    ImportMutation,
    ApiKeyMutation,
    AccountMutation,
);

/// Das Lineal-Schema (ohne Subscriptions; Echtzeit läuft über `/sync`).
pub type LinealSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

/// Baut das Schema mit `Arc<App>` als Context-Daten. `RequestCtx` setzt die Auth-Middleware
/// pro Request (`Request::data`).
pub fn build_schema(app: Arc<App>) -> LinealSchema {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        EmptySubscription,
    )
    .data(app)
    // Zweite Verteidigungslinie neben der eigenen Komplexitätsprüfung (middleware::gql).
    .limit_depth(MAX_QUERY_DEPTH)
    .limit_recursive_depth(MAX_QUERY_DEPTH * 2)
    .finish()
}

/// Maximale Verschachtelungstiefe einer Abfrage.
pub const MAX_QUERY_DEPTH: usize = 16;

/// SDL ohne laufende App (für `lineal schema-export`).
pub fn schema_sdl() -> String {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        EmptySubscription,
    )
    .finish()
    .sdl()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_testkit::fake_app;

    #[test]
    fn schema_builds_with_query_and_mutation_roots() {
        // Bereichsunabhängig, damit Arbeitspakete ihre Platzhalter entfernen können.
        let sdl = schema_sdl();
        assert!(sdl.contains("type Query {"));
        assert!(sdl.contains("type Mutation {"));
    }

    #[tokio::test]
    async fn schema_executes_a_trivial_query() {
        let schema = build_schema(fake_app());
        let res = schema.execute("{ __typename }").await;
        assert!(res.errors.is_empty(), "{:?}", res.errors);
    }
}
