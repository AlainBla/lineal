//! Repository-Port `CommentRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `comment`.
#[async_trait]
pub trait CommentRepo: Send {}
