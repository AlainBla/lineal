//! `NotificationRepo` für `PgTx` (Stub; Methoden ergänzt das zuständige Arbeitspaket).

use crate::store::PgTx;
use async_trait::async_trait;
use lineal_app::ports::repos::NotificationRepo;

#[async_trait]
impl NotificationRepo for PgTx {}
