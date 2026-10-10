//! `AuthRepo` für `PgTx` (Stub; Implementierung durch WP02).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::AppResult;
use lineal_app::ports::repos::{AuthRepo, Principal};

#[async_trait]
impl AuthRepo for PgTx {
    async fn find_principal_by_token_hash(&mut self, _hash: &str) -> AppResult<Option<Principal>> {
        Ok(None)
    }
}
