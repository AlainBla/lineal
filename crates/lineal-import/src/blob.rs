//! Ablage heruntergeladener Dateien im Blob-Store von Lineal.
//!
//! `AppBlobSink` schreibt über `BlobStore::put_bytes` direkt in den Store der `App`. Es gibt keine
//! Netzwerkverbindung und nie eine Adresse aus importierten Daten.

use async_trait::async_trait;
use lineal_app::{App, AppResult};
use std::collections::HashMap;
use std::sync::Mutex;

/// Senke für Dateien.
#[async_trait]
pub trait BlobSink: Send + Sync {
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()>;
}

/// Schreibt direkt über den `BlobStore`-Port der `App` (kein HTTP-Umweg über `LINEAL_BASE_URL`,
/// die im Worker-Container oft nicht erreichbar ist). Größen- und Typprüfung (Magic-Bytes) wie
/// beim Upload macht der Store.
pub struct AppBlobSink<'a> {
    pub app: &'a App,
}

#[async_trait]
impl BlobSink for AppBlobSink<'_> {
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()> {
        self.app.blobs.put_bytes(key, content_type, bytes).await
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
