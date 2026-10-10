//! `ImportRepo` für `PgTx` (Stub; Methoden ergänzt das zuständige Arbeitspaket).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::ports::repos::ImportRepo;

#[async_trait]
impl ImportRepo for PgTx {}
