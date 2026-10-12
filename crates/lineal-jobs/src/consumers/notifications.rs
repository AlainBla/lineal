//! Outbox-Konsument `notifications` (WP08/WP32): Fan-out der Domain-Events in Inbox-Notifications
//! (docs/10 §4). Die Logik steht in `lineal_app::usecases::notification::fanout`.
//!
//! View-Abos (docs/05 §4.7): [`NotificationsWithViewsConsumer`] ist derselbe Konsument (Name
//! `notifications`) mit einer [`ViewSubscriptionSource`]; `registry::consumers()` trägt ihn mit
//! `ViewSubscriptions` (WP35) ein. [`NotificationsConsumer`] (ohne Quelle) bleibt für Tests.

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::usecases::notification::views::ViewSubscriptionSource;
use lineal_app::{App, AppResult};
use std::sync::Arc;

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

/// Wie [`NotificationsConsumer`], zusätzlich mit View-Abos.
pub struct NotificationsWithViewsConsumer {
    pub views: Arc<dyn ViewSubscriptionSource>,
}

#[async_trait]
impl EventConsumer for NotificationsWithViewsConsumer {
    fn name(&self) -> &'static str {
        "notifications"
    }

    async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()> {
        app.fan_out_event_with(ev, Some(self.views.as_ref()))
            .await
            .map(|_| ())
    }
}
