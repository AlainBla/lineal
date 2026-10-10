//! Repository-Port `AuthRepo`.

use crate::context::{Actor, Scopes};
use crate::error::AppResult;
use async_trait::async_trait;
use lineal_domain::ids::OrgId;

/// Aufgelöster Aufrufer eines Tokens (Session, API-Key oder OAuth-Token).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Principal {
    pub org_id: OrgId,
    pub actor: Actor,
    pub scopes: Scopes,
}

#[async_trait]
pub trait AuthRepo: Send {
    /// Sucht den Aufrufer zu einem SHA-256-Token-Hash (hex). Implementierung: WP02.
    async fn find_principal_by_token_hash(&mut self, hash: &str) -> AppResult<Option<Principal>>;
}
