//! `ProjectRepo` für `PgTx` (Stub; Methoden ergänzt das zuständige Arbeitspaket).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::ports::repos::ProjectRepo;

#[async_trait]
impl ProjectRepo for PgTx {}
