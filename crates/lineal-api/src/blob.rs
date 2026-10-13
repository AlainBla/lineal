//! Lokaler Blob-Store mit HMAC-signierten URLs (docs/09 §1.7, docs/10 §11.4 Nr. 5) und die
//! Endpunkte `PUT /uploads/{token}` und `GET /files/{token}`.
//!
//! Token = `base64url(json claims) "." base64url(HMAC-SHA256)`. Claims enthalten Operation,
//! Schlüssel, Content-Type (Upload) und Ablaufzeit; signiert wird mit dem Session-Secret
//! (mit Domain-Präfix). Auf der Platte liegt jede Datei unter dem SHA-256 ihres Schlüssels
//! (`data/<hash>` plus `meta/<hash>.json`), damit Schlüssel nie Pfade beeinflussen.

use crate::error::ApiError;
use async_trait::async_trait;
use axum::Extension;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use lineal_app::ports::services::BlobInfo;
use lineal_app::ports::{BlobStore, Clock};
use lineal_app::{App, AppError, AppResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Gültigkeit von Upload-URLs (09 §1.7).
pub const PUT_TTL: Duration = Duration::from_secs(60);
/// Gültigkeit von Download-URLs.
pub const GET_TTL: Duration = Duration::from_secs(5 * 60);
/// Default-Obergrenze für Uploads (09 §1.7: 50 MB).
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
const MAX_KEY_LEN: usize = 1024;
const DOMAIN: &str = "lineal-blob-v1.";

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct Claims {
    /// `put` oder `get`.
    op: String,
    key: String,
    /// Erwarteter Content-Type (nur Upload).
    ct: Option<String>,
    /// Ablauf (Unix-Sekunden).
    exp: i64,
    /// Reservierte Größe in Bytes (nur Upload): der PUT darf sie nicht überschreiten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct Meta {
    key: String,
    content_type: String,
    size: u64,
}

/// `BlobStore` auf dem lokalen Dateisystem; die URLs zeigen auf `{base_url}/uploads|files/{token}`.
#[derive(Clone)]
pub struct LocalBlobStore {
    dir: PathBuf,
    secret: Vec<u8>,
    base_url: String,
    clock: Arc<dyn Clock>,
    max_bytes: u64,
}

impl LocalBlobStore {
    pub fn new(
        dir: impl Into<PathBuf>,
        secret: impl AsRef<[u8]>,
        base_url: impl Into<String>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            dir: dir.into(),
            secret: secret.as_ref().to_vec(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            clock,
            max_bytes: DEFAULT_MAX_UPLOAD_BYTES,
        }
    }

    /// Store passend zur Konfiguration der `App` (so bauen auch die Endpunkte ihn).
    pub fn from_app(app: &App) -> Self {
        Self::new(
            &app.config.blob_dir,
            &app.config.session_secret,
            &app.config.base_url,
            app.clock.clone(),
        )
    }

    #[must_use]
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    fn mac(&self, payload: &str) -> HmacSha256 {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(DOMAIN.as_bytes());
        mac.update(payload.as_bytes());
        mac
    }

    fn sign(&self, claims: &Claims) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap_or_default());
        let sig = self.mac(&payload).finalize().into_bytes();
        format!("{payload}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    fn verify(&self, token: &str, op: &str) -> AppResult<Claims> {
        let forbidden = || AppError::forbidden("Invalid signature.");
        let (payload, sig) = token.split_once('.').ok_or_else(forbidden)?;
        let sig = URL_SAFE_NO_PAD.decode(sig).map_err(|_| forbidden())?;
        // Vergleich in konstanter Zeit (`verify_slice`).
        self.mac(payload)
            .verify_slice(&sig)
            .map_err(|_| forbidden())?;
        let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| forbidden())?;
        let claims: Claims = serde_json::from_slice(&bytes).map_err(|_| forbidden())?;
        if claims.op != op {
            return Err(forbidden());
        }
        if claims.exp < self.clock.now().timestamp() {
            return Err(AppError::forbidden("Signed URL has expired."));
        }
        Ok(claims)
    }

    fn url(&self, route: &str, claims: &Claims) -> String {
        format!("{}/{route}/{}", self.base_url, self.sign(claims))
    }

    fn expiry(&self, ttl: Duration) -> i64 {
        let now: DateTime<Utc> = self.clock.now();
        now.timestamp() + i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX / 2)
    }

    fn paths(&self, key: &str) -> (PathBuf, PathBuf) {
        let hash = hex(&Sha256::digest(key.as_bytes()));
        (
            self.dir.join("data").join(&hash),
            self.dir.join("meta").join(format!("{hash}.json")),
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn check_key(key: &str) -> AppResult<()> {
    if key.is_empty() || key.len() > MAX_KEY_LEN || key.contains('\0') {
        return Err(AppError::invalid_request("Invalid blob key."));
    }
    Ok(())
}

#[async_trait]
impl BlobStore for LocalBlobStore {
    async fn head(&self, key: &str) -> AppResult<Option<BlobInfo>> {
        check_key(key)?;
        let (data_path, meta_path) = self.paths(key);
        let meta: Meta = match tokio::fs::read(&meta_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| AppError::internal("blob storage: corrupt metadata"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(e)),
        };
        // Metadaten werden nach den Daten geschrieben; maßgeblich ist die tatsächliche Dateigröße.
        match tokio::fs::metadata(&data_path).await {
            Ok(m) if meta.key == key => Ok(Some(BlobInfo {
                size: m.len(),
                content_type: meta.content_type,
            })),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(e)),
        }
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        check_key(key)?;
        let (data_path, meta_path) = self.paths(key);
        // Metadaten zuerst: ohne sie gilt das Objekt für `head` schon als nicht vorhanden.
        for path in [meta_path, data_path] {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(e)),
            }
        }
        Ok(())
    }

    async fn signed_put_url(
        &self,
        key: &str,
        content_type: &str,
        ttl: Duration,
    ) -> AppResult<String> {
        check_key(key)?;
        Ok(self.url(
            "uploads",
            &Claims {
                op: "put".into(),
                key: key.into(),
                ct: Some(content_type.into()),
                exp: self.expiry(ttl),
                max: None,
            },
        ))
    }

    async fn signed_put_url_sized(
        &self,
        key: &str,
        content_type: &str,
        max_size: u64,
        ttl: Duration,
    ) -> AppResult<String> {
        check_key(key)?;
        Ok(self.url(
            "uploads",
            &Claims {
                op: "put".into(),
                key: key.into(),
                ct: Some(content_type.into()),
                exp: self.expiry(ttl),
                max: Some(max_size),
            },
        ))
    }

    async fn signed_get_url(&self, key: &str, ttl: Duration) -> AppResult<String> {
        check_key(key)?;
        Ok(self.url(
            "files",
            &Claims {
                op: "get".into(),
                key: key.into(),
                ct: None,
                exp: self.expiry(ttl),
                max: None,
            },
        ))
    }
}

fn essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn io_err(e: std::io::Error) -> AppError {
    AppError::internal(format!("blob storage: {e}"))
}

async fn write_upload(store: &LocalBlobStore, claims: &Claims, body: Body) -> AppResult<u64> {
    let (data_path, meta_path) = store.paths(&claims.key);
    for p in [&data_path, &meta_path] {
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io_err)?;
        }
    }
    let tmp = store.dir.join(format!("{}.tmp", Uuid::now_v7().simple()));
    let limit = claims
        .max
        .map_or(store.max_bytes, |m| m.min(store.max_bytes));
    let ct = essence(claims.ct.as_deref().unwrap_or_default());
    let result = stream_to_file(&tmp, body, limit, &ct).await;
    let size = match result {
        Ok(size) => size,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };
    tokio::fs::rename(&tmp, &data_path).await.map_err(io_err)?;
    let meta = Meta {
        key: claims.key.clone(),
        content_type: claims.ct.clone().unwrap_or_default(),
        size,
    };
    tokio::fs::write(&meta_path, serde_json::to_vec(&meta).unwrap_or_default())
        .await
        .map_err(io_err)?;
    Ok(size)
}

/// Prüft die Anfangsbytes gegen den deklarierten Typ (Bilder, PDF). Andere Typen sind frei.
fn magic_matches(content_type: &str, head: &[u8]) -> bool {
    match content_type {
        "image/png" => head.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        "image/webp" => head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP",
        "application/pdf" => head.starts_with(b"%PDF-"),
        _ => true,
    }
}

async fn stream_to_file(
    tmp: &FsPath,
    mut body: Body,
    max_bytes: u64,
    content_type: &str,
) -> AppResult<u64> {
    let mut file = tokio::fs::File::create(tmp).await.map_err(io_err)?;
    let mut size: u64 = 0;
    let mut head: Vec<u8> = Vec::with_capacity(16);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| AppError::invalid_request("Could not read request body."))?;
        if let Ok(data) = frame.into_data() {
            size += data.len() as u64;
            if size > max_bytes {
                return Err(AppError::limit_exceeded(format!(
                    "Upload exceeds the maximum of {max_bytes} bytes."
                ))
                .with_details(serde_json::json!({ "maxBytes": max_bytes })));
            }
            if head.len() < 16 {
                let take = (16 - head.len()).min(data.len());
                head.extend_from_slice(&data[..take]);
            }
            file.write_all(&data).await.map_err(io_err)?;
        }
    }
    file.flush().await.map_err(io_err)?;
    if !magic_matches(content_type, &head) {
        return Err(AppError::invalid_request(
            "The file content does not match the declared content type.",
        ));
    }
    Ok(size)
}

/// `PUT /uploads/{token}`: Inhalt speichern. Content-Type muss zum signierten passen.
pub async fn upload(
    Extension(app): Extension<Arc<App>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let store = LocalBlobStore::from_app(&app);
    let claims = store.verify(&token, "put")?;
    let sent = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if essence(sent) != essence(claims.ct.as_deref().unwrap_or_default()) {
        return Err(
            AppError::invalid_request("Content-Type does not match the signed URL.").into(),
        );
    }
    let limit = claims
        .max
        .map_or(store.max_bytes, |m| m.min(store.max_bytes));
    if let Some(len) = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        && len > limit
    {
        return Err(
            AppError::limit_exceeded("Upload exceeds the reserved size.")
                .with_details(serde_json::json!({ "maxBytes": limit }))
                .into(),
        );
    }
    write_upload(&store, &claims, body).await?;
    Ok(StatusCode::OK)
}

/// Typen, die der Browser inline darstellen darf (kein HTML/SVG/Skript).
fn inline_safe(content_type: &str) -> bool {
    matches!(
        essence(content_type).as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "application/pdf"
            | "text/plain"
            | "video/mp4"
            | "video/webm"
            | "audio/mpeg"
            | "audio/ogg"
    )
}

fn filename_of(key: &str) -> String {
    let name = key.rsplit('/').next().unwrap_or("file");
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "file".into()
    } else {
        clean
    }
}

/// `GET /files/{token}`: Inhalt ausliefern (sandboxed, `nosniff`).
pub async fn download(
    Extension(app): Extension<Arc<App>>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    let store = LocalBlobStore::from_app(&app);
    let claims = store.verify(&token, "get")?;
    let (data_path, meta_path) = store.paths(&claims.key);
    let not_found = || AppError::not_found("File not found.");
    let meta: Meta = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| not_found())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found().into()),
        Err(e) => return Err(io_err(e).into()),
    };
    let file = match tokio::fs::File::open(&data_path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found().into()),
        Err(e) => return Err(io_err(e).into()),
    };
    let disposition = format!(
        "{}; filename=\"{}\"",
        if inline_safe(&meta.content_type) {
            "inline"
        } else {
            "attachment"
        },
        filename_of(&meta.key)
    );
    let mut res = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let h = res.headers_mut();
    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&meta.content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(CONTENT_LENGTH, HeaderValue::from(meta.size));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(CONTENT_DISPOSITION, v);
    }
    // Nutzerinhalt darf nie im Ursprung der App Skripte ausführen.
    h.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct At(i64);
    impl Clock for At {
        fn now(&self) -> DateTime<Utc> {
            DateTime::from_timestamp(self.0, 0).unwrap_or_default()
        }
    }

    fn store(now: i64) -> LocalBlobStore {
        LocalBlobStore::new(
            "/nonexistent",
            "change-me-32-bytes-minimum-xxxxxxxx",
            "http://localhost:8080/",
            Arc::new(At(now)),
        )
    }

    fn token_of(url: &str) -> &str {
        url.rsplit('/').next().unwrap_or_default()
    }

    #[tokio::test]
    async fn signed_urls_verify_expire_and_reject_tampering() {
        let s = store(1_000);
        let put = s
            .signed_put_url("org/a b.png", "image/png", PUT_TTL)
            .await
            .unwrap();
        assert!(put.starts_with("http://localhost:8080/uploads/"));
        let claims = s.verify(token_of(&put), "put").unwrap();
        assert_eq!(claims.key, "org/a b.png");
        assert_eq!(claims.exp, 1_060);
        // falsche Operation
        assert!(s.verify(token_of(&put), "get").is_err());
        // abgelaufen
        let late = store(1_061);
        assert_eq!(
            late.verify(token_of(&put), "put").unwrap_err().kind,
            lineal_app::ErrorKind::Forbidden
        );
        // manipulierter Payload
        let (payload, sig) = token_of(&put).split_once('.').unwrap();
        let forged = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Claims {
                op: "put".into(),
                key: "other".into(),
                ct: None,
                exp: 9_999_999,
            })
            .unwrap(),
        );
        assert_ne!(forged, payload);
        assert!(s.verify(&format!("{forged}.{sig}"), "put").is_err());
        // anderes Secret
        let other = LocalBlobStore::new(
            "/x",
            "another-secret-32-bytes-minimum-xxxxx",
            "http://x",
            Arc::new(At(1_000)),
        );
        assert!(other.verify(token_of(&put), "put").is_err());
        assert!(s.verify("garbage", "put").is_err());
    }

    #[test]
    fn keys_never_reach_the_file_system_directly() {
        let s = store(0);
        let (data, meta) = s.paths("../../etc/passwd");
        assert!(data.starts_with("/nonexistent/data"));
        assert!(!data.to_string_lossy().contains(".."));
        assert!(meta.to_string_lossy().ends_with(".json"));
        assert_eq!(filename_of("a/b/../ev il.html"), "ev_il.html");
    }

    #[tokio::test]
    async fn upload_stops_at_the_size_limit_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("lineal-blob-unit-{}", Uuid::now_v7()));
        let s = LocalBlobStore::new(
            &dir,
            "change-me-32-bytes-minimum-xxxxxxxx",
            "http://x",
            Arc::new(At(0)),
        )
        .with_max_bytes(8);
        let claims = |key: &str| Claims {
            op: "put".into(),
            key: key.into(),
            ct: Some("text/plain".into()),
            exp: 1,
        };
        let err = write_upload(&s, &claims("big"), Body::from(vec![0u8; 20]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, lineal_app::ErrorKind::LimitExceeded);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(
            leftovers
                .iter()
                .all(|n| !n.to_string_lossy().ends_with(".tmp")),
            "{leftovers:?}"
        );
        assert_eq!(
            write_upload(&s, &claims("ok"), Body::from(vec![0u8; 8]))
                .await
                .unwrap(),
            8
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
