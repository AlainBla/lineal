//! Repository-Port `OverviewRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `overview`.
#[async_trait]
pub trait OverviewRepo: Send {}
