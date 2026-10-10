//! Repository-Port `NotificationRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `notification`.
#[async_trait]
pub trait NotificationRepo: Send {}
