//! Technische Dienste hinter Ports (Zeit, IDs, Blobs, Suche, Passwörter, Mail).

use crate::error::AppResult;
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

/// Objektspeicher mit signierten URLs.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Signierte Upload-URL (PUT).
    async fn signed_put_url(&self, key: &str, content_type: &str, ttl: Duration) -> AppResult<String>;
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
