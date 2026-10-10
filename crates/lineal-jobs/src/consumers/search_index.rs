//! Outbox-Konsument `search_index` (WP11). Im Scaffold ein No-op.

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};

pub struct SearchIndexConsumer;

#[async_trait]
impl EventConsumer for SearchIndexConsumer {
    fn name(&self) -> &'static str {
        "search_index"
    }

    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        Ok(())
    }
}
