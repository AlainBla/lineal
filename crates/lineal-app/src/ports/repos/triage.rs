//! Repository-Port `TriageRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `triage`.
#[async_trait]
pub trait TriageRepo: Send {}
