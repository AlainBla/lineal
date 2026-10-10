//! Zentrale Fassade `App`, die Adapter benutzen. Use-Cases stehen als `impl App` in `usecases/*`.

use crate::ports::{BlobStore, Clock, IdGen, Mailer, PasswordHasher, SearchIndex, Store};
use std::path::PathBuf;
use std::sync::Arc;

/// Konfiguration, die der Application-Layer kennt (aus der Umgebung gelesen vom Server).
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub base_url: String,
    pub session_secret: String,
    pub blob_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080".into(),
            session_secret: "change-me-32-bytes-minimum-xxxxxxxx".into(),
            blob_dir: PathBuf::from("./var/blobs"),
        }
    }
}

/// Fassade über Ports und Konfiguration.
pub struct App {
    pub store: Arc<dyn Store>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub blobs: Arc<dyn BlobStore>,
    pub search: Arc<dyn SearchIndex>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub mailer: Arc<dyn Mailer>,
    pub config: AppConfig,
}
