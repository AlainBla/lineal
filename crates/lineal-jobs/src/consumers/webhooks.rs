//! Outbox-Konsument `webhooks` (WP16). Im Scaffold ein No-op.

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};

pub struct WebhooksConsumer;

#[async_trait]
impl EventConsumer for WebhooksConsumer {
    fn name(&self) -> &'static str {
        "webhooks"
    }

    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        Ok(())
    }
}
