//! GraphQL-Teilobjekte des Bereichs `comment` (WP09).
//!
//! Das Platzhalterfeld `_commentReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct CommentQuery;

#[Object]
impl CommentQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_commentReady")]
    async fn comment_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct CommentMutation;

#[Object]
impl CommentMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_commentMutationReady")]
    async fn comment_mutation_ready(&self) -> bool {
        true
    }
}
