//! Repository-Port `ProjectRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `project`.
#[async_trait]
pub trait ProjectRepo: Send {}
