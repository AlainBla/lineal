//! Repository-Port `TeamRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `team`.
#[async_trait]
pub trait TeamRepo: Send {}
