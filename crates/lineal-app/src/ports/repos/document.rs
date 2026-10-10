//! Repository-Port `DocumentRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `document`.
#[async_trait]
pub trait DocumentRepo: Send {}
