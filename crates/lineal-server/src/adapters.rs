//! Konkrete Adapter für die Ports (WP03 ersetzt Blob-Store und Hasher): System-Clock,
//! UUID-v7-IdGen, Dummy-BlobStore, Dummy-Hasher, leere Suche, Log-Mailer.

use crate::config::Config;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::{
    BlobStore, Clock, IdGen, Mail, Mailer, PasswordHasher, SearchHit, SearchIndex, Store,
};
use lineal_app::{App, AppConfig, AppError, AppResult};
use lineal_domain::ids::OrgId;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct UuidV7IdGen;

impl IdGen for UuidV7IdGen {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// Platzhalter bis WP03 den lokalen Blob-Store mit signierten URLs liefert.
pub struct DummyBlobStore;

#[async_trait]
impl BlobStore for DummyBlobStore {
    async fn signed_put_url(
        &self,
        _key: &str,
        _content_type: &str,
        _ttl: Duration,
    ) -> AppResult<String> {
        Err(AppError::internal("not implemented"))
    }
    async fn signed_get_url(&self, _key: &str, _ttl: Duration) -> AppResult<String> {
        Err(AppError::internal("not implemented"))
    }
}

/// Platzhalter bis WP03 Argon2 einbaut.
pub struct DummyHasher;

impl PasswordHasher for DummyHasher {
    fn hash(&self, _password: &str) -> AppResult<String> {
        Err(AppError::internal("not implemented"))
    }
    fn verify(&self, _password: &str, _hash: &str) -> AppResult<bool> {
        Err(AppError::internal("not implemented"))
    }
}

/// Leere Suche bis WP11.
pub struct NoSearch;

#[async_trait]
impl SearchIndex for NoSearch {
    async fn search(&self, _org: OrgId, _query: &str, _limit: u32) -> AppResult<Vec<SearchHit>> {
        Ok(vec![])
    }
}

/// Schreibt Mails nur ins Log (lokale Entwicklung).
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: Mail) -> AppResult<()> {
        tracing::info!(to = %mail.to, subject = %mail.subject, "mail (not sent)");
        Ok(())
    }
}

/// Verdrahtet die `App` mit allen Adaptern.
pub fn build_app(store: Arc<dyn Store>, config: &Config) -> Arc<App> {
    Arc::new(App {
        store,
        clock: Arc::new(SystemClock),
        ids: Arc::new(UuidV7IdGen),
        blobs: Arc::new(DummyBlobStore),
        search: Arc::new(NoSearch),
        hasher: Arc::new(DummyHasher),
        mailer: Arc::new(LogMailer),
        config: AppConfig {
            base_url: config.base_url.clone(),
            session_secret: config.session_secret.clone(),
            blob_dir: config.blob_dir.clone(),
        },
    })
}
