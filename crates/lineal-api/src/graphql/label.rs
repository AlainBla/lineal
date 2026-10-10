//! GraphQL-Teilobjekte des Bereichs `label` (WP06).
//!
//! Das Platzhalterfeld `_labelReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct LabelQuery;

#[Object]
impl LabelQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_labelReady")]
    async fn label_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct LabelMutation;

#[Object]
impl LabelMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_labelMutationReady")]
    async fn label_mutation_ready(&self) -> bool {
        true
    }
}
