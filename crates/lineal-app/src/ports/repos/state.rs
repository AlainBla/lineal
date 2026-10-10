//! Repository-Port `WorkflowStateRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `state`.
#[async_trait]
pub trait WorkflowStateRepo: Send {}
