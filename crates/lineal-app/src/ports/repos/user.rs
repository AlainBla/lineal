//! Repository-Port `UserRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `user`.
#[async_trait]
pub trait UserRepo: Send {}
