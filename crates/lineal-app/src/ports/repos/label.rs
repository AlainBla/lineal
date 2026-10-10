//! Repository-Port `LabelRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `label`.
#[async_trait]
pub trait LabelRepo: Send {}
