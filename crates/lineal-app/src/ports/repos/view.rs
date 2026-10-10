//! Repository-Port `ViewRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `view`.
#[async_trait]
pub trait ViewRepo: Send {}
