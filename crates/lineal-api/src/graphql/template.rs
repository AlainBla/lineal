//! GraphQL-Teilobjekte des Bereichs `template` (WP10).
//!
//! Das Platzhalterfeld `_templateReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct TemplateQuery;

#[Object]
impl TemplateQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_templateReady")]
    async fn template_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct TemplateMutation;

#[Object]
impl TemplateMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_templateMutationReady")]
    async fn template_mutation_ready(&self) -> bool {
        true
    }
}
