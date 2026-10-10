//! Repository-Port `OrgRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `org`.
#[async_trait]
pub trait OrgRepo: Send {}
