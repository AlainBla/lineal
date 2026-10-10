//! Outbox-Konsument `notifications` (WP08). Im Scaffold ein No-op.

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};

pub struct NotificationsConsumer;

#[async_trait]
impl EventConsumer for NotificationsConsumer {
    fn name(&self) -> &'static str {
        "notifications"
    }

    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        Ok(())
    }
}
