//! Repository-Port `CycleRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `cycle`.
#[async_trait]
pub trait CycleRepo: Send {}
