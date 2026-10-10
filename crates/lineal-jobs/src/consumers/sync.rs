//! Outbox-Konsument `sync` (WP15). Im Scaffold ein No-op.

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};

pub struct SyncConsumer;

#[async_trait]
impl EventConsumer for SyncConsumer {
    fn name(&self) -> &'static str {
        "sync"
    }

    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        Ok(())
    }
}
