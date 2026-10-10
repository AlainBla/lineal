//! Repository-Port `ImportRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `import`.
#[async_trait]
pub trait ImportRepo: Send {}
