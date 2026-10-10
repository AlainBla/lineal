//! Outbox-Konsument `notifications` (WP08): Fan-out der Domain-Events in Inbox-Notifications
//! (docs/10 §4). Die Logik steht in `lineal_app::usecases::notification::fanout`.

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

    async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()> {
        app.fan_out_event(ev).await.map(|_| ())
    }
}
