//! Repository-Port `IssueRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `issue`.
#[async_trait]
pub trait IssueRepo: Send {}
