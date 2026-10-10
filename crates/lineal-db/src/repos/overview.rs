//! `OverviewRepo` für `PgTx` (Stub; Methoden ergänzt das zuständige Arbeitspaket).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::ports::repos::OverviewRepo;

#[async_trait]
impl OverviewRepo for PgTx {}
