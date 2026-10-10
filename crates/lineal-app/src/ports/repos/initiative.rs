//! Repository-Port `InitiativeRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `initiative`.
#[async_trait]
pub trait InitiativeRepo: Send {}
