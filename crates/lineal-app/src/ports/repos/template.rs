//! Repository-Port `TemplateRepo` (Methoden ergänzt das zuständige Arbeitspaket).

use async_trait::async_trait;

/// Persistenzzugriff des Bereichs `template`.
#[async_trait]
pub trait TemplateRepo: Send {}
