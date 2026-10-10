//! Konkrete Adapter für die Ports: System-Clock, UUID-v7-IdGen, lokaler BlobStore,
//! Argon2-Hasher, leere Suche, Log-Mailer.

use crate::config::Config;
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::{
    Clock, IdGen, Mail, Mailer, PasswordHasher, SearchHit, SearchIndex, Store,
};
use lineal_app::{App, AppConfig, AppError, AppResult};
use lineal_domain::ids::OrgId;
use std::sync::Arc;
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

/// Lokaler Blob-Store (Verzeichnis `LINEAL_BLOB_DIR`, HMAC-signierte URLs). Die Endpunkte
/// `/uploads/{token}` und `/files/{token}` liegen in `lineal-api::blob`.
pub type LocalBlobStore = lineal_api::blob::LocalBlobStore;

/// Argon2id-Passwort-Hashing (PHC-String mit zufälligem Salt, Standardparameter der Crate).
#[derive(Default)]
pub struct Argon2Hasher {
    argon2: Argon2<'static>,
}

impl PasswordHasher for Argon2Hasher {
    fn hash(&self, password: &str) -> AppResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        self.argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::internal(format!("password hashing failed: {e}")))
    }

    /// Falsches Passwort oder unlesbarer Hash ergeben `Ok(false)` (kein Hinweis für Angreifer).
    fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        let Ok(parsed) = PasswordHash::new(hash) else {
            return Ok(false);
        };
        Ok(self
            .argon2
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
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
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Arc::new(App {
        store,
        blobs: Arc::new(LocalBlobStore::new(
            &config.blob_dir,
            &config.session_secret,
            &config.base_url,
            clock.clone(),
        )),
        clock,
        ids: Arc::new(UuidV7IdGen),
        search: Arc::new(NoSearch),
        hasher: Arc::new(Argon2Hasher::default()),
        mailer: Arc::new(LogMailer),
        config: AppConfig {
            base_url: config.base_url.clone(),
            session_secret: config.session_secret.clone(),
            blob_dir: config.blob_dir.clone(),
            allowed_origins: config.allowed_origins.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn argon2_roundtrip_with_salt_and_garbage_hash() {
        let h = Argon2Hasher::default();
        let a = h.hash("correct horse").unwrap();
        let b = h.hash("correct horse").unwrap();
        assert!(a.starts_with("$argon2id$"));
        assert_ne!(a, b, "salt must differ");
        assert!(h.verify("correct horse", &a).unwrap());
        assert!(!h.verify("wrong", &a).unwrap());
        assert!(!h.verify("x", "not-a-phc-string").unwrap());
    }

    #[test]
    fn clock_and_ids_are_real() {
        let before = Utc::now();
        assert!(SystemClock.now() >= before);
        let (a, b) = (UuidV7IdGen.new_uuid(), UuidV7IdGen.new_uuid());
        assert_eq!(a.get_version_num(), 7);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn build_app_wires_a_working_blob_store() {
        let cfg = Config::from_lookup(|k| (k == "LINEAL_DEV").then(|| "1".into())).unwrap();
        let app = build_app(Arc::new(lineal_testkit::FakeStore::new()), &cfg);
        let url = app
            .blobs
            .signed_put_url("k", "text/plain", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(url.starts_with("http://localhost:8080/uploads/"));
        assert!(app.hasher.hash("x").unwrap().starts_with("$argon2id$"));
    }
}
