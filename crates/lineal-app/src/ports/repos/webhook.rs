//! Repository-Port `WebhookRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `webhook`.
#[async_trait]
pub trait WebhookRepo: Send {}
