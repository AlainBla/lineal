//! Repository-Port `AttachmentRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `attachment`.
#[async_trait]
pub trait AttachmentRepo: Send {}
