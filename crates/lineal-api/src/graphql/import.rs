//! GraphQL-Teilobjekte des Bereichs `import` (WP18).
//!
//! Das Platzhalterfeld `_importReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct ImportQuery;

#[Object]
impl ImportQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_importReady")]
    async fn import_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct ImportMutation;

#[Object]
impl ImportMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_importMutationReady")]
    async fn import_mutation_ready(&self) -> bool {
        true
    }
}
