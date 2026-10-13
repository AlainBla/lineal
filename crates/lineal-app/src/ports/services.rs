//! Technische Dienste hinter Ports (Zeit, IDs, Blobs, Suche, Passwörter, Mail).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::OrgId;
use std::time::Duration;
use uuid::Uuid;

/// Zeitquelle (testbar).
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// ID-Generator (UUID v7 für neue Datensätze).
pub trait IdGen: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}

/// Metadaten eines gespeicherten Objekts (`BlobStore::head`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    pub size: u64,
    pub content_type: String,
}

/// Objektspeicher mit signierten URLs.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Existenz und Größe eines Objekts (für `create_attachment_from_upload`).
    /// `None`, wenn es nicht (vollständig) hochgeladen wurde. Zusatz aus WP09.
    async fn head(&self, key: &str) -> AppResult<Option<BlobInfo>> {
        let _ = key;
        Err(AppError::internal("blob head not supported by this store"))
    }

    /// Löscht das Objekt endgültig. Ein fehlendes Objekt ist kein Fehler (idempotent).
    /// Der Standard ist ein No-op für Stores ohne Löschfunktion (Fakes in Tests).
    async fn delete(&self, key: &str) -> AppResult<()> {
        let _ = key;
        Ok(())
    }

    /// Signierte Upload-URL (PUT).
    async fn signed_put_url(
        &self,
        key: &str,
        content_type: &str,
        ttl: Duration,
    ) -> AppResult<String>;
    /// Wie [`Self::signed_put_url`], mit der reservierten Größe: der Upload darf sie nicht
    /// überschreiten. Der Standard ignoriert die Größe (Stores ohne Durchsetzung, Fakes).
    async fn signed_put_url_sized(
        &self,
        key: &str,
        content_type: &str,
        max_size: u64,
        ttl: Duration,
    ) -> AppResult<String> {
        let _ = max_size;
        self.signed_put_url(key, content_type, ttl).await
    }
    /// Signierte Download-URL (GET).
    async fn signed_get_url(&self, key: &str, ttl: Duration) -> AppResult<String>;
}

/// Treffer der Volltextsuche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Entitätsname (`Issue`, `Project`, `Document` …).
    pub model_name: String,
    pub id: Uuid,
}

/// Suchindex (Standard: PostgreSQL-Volltext, später austauschbar).
#[async_trait]
pub trait SearchIndex: Send + Sync {
    async fn search(&self, org: OrgId, query: &str, limit: u32) -> AppResult<Vec<SearchHit>>;
}

/// Passwort-Hashing (Argon2 in WP03).
pub trait PasswordHasher: Send + Sync {
    fn hash(&self, password: &str) -> AppResult<String>;
    fn verify(&self, password: &str, hash: &str) -> AppResult<bool>;
}

/// Eine zu versendende E-Mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// E-Mail-Versand.
#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, mail: Mail) -> AppResult<()>;
}
