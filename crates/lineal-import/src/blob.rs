//! Ablage heruntergeladener Dateien im Blob-Store von Lineal.
//!
//! Der `BlobStore`-Port kennt nur signierte URLs; `AppBlobSink` holt eine signierte Upload-URL und
//! lädt per HTTP `PUT` hoch (wie jeder Client). Das Ziel ist ausschließlich die eigene
//! `base_url`/der eigene Blob-Store, nie eine Adresse aus importierten Daten.

use async_trait::async_trait;
use lineal_app::{App, AppError, AppResult};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// Senke für Dateien.
#[async_trait]
pub trait BlobSink: Send + Sync {
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()>;
}

/// Lädt über die signierte Upload-URL des `App`-Blob-Stores hoch.
pub struct AppBlobSink<'a> {
    pub app: &'a App,
}

#[async_trait]
impl BlobSink for AppBlobSink<'_> {
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()> {
        let url = self
            .app
            .blobs
            .signed_put_url(key, content_type, Duration::from_secs(60))
            .await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| AppError::internal("http client setup failed"))?;
        let resp = client
            .put(&url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|_| AppError::internal("file upload to the blob store failed"))?;
        if !resp.status().is_success() {
            return Err(AppError::internal(format!(
                "file upload to the blob store failed (HTTP {})",
                resp.status().as_u16()
            )));
        }
        Ok(())
    }
}

/// Speicher im Arbeitsspeicher (Tests).
#[derive(Default)]
pub struct MemoryBlobSink {
    files: Mutex<HashMap<String, (String, Vec<u8>)>>,
}

impl MemoryBlobSink {
    /// Anzahl gespeicherter Dateien.
    pub fn len(&self) -> usize {
        self.files.lock().map(|f| f.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inhalt und Content-Type zu einem Schlüssel.
    pub fn get(&self, key: &str) -> Option<(String, Vec<u8>)> {
        self.files.lock().ok()?.get(key).cloned()
    }
}

#[async_trait]
impl BlobSink for MemoryBlobSink {
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()> {
        if let Ok(mut f) = self.files.lock() {
            f.insert(key.to_owned(), (content_type.to_owned(), bytes));
        }
        Ok(())
    }
}
