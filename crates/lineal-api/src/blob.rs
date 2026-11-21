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
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
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
    /// Ursprünglicher Dateiname (nur Upload, LIN-85): wird an der Datei hinterlegt und beim
    /// Download als `Content-Disposition`-Name verwendet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fname: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Meta {
    key: String,
    content_type: String,
    size: u64,
    /// Ursprünglicher Dateiname aus dem Upload (LIN-85). Fehlt bei Altbeständen und bei
    /// `put_bytes`-Importen → Download fällt auf den Schlüsselnamen (`<uuid>.<ext>`) zurück.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
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

    async fn read(&self, key: &str, max_bytes: u64) -> AppResult<Option<Vec<u8>>> {
        let Some(info) = self.head(key).await? else {
            return Ok(None);
        };
        if info.size > max_bytes {
            return Ok(None);
        }
        let (data_path, _) = self.paths(key);
        match tokio::fs::read(&data_path).await {
            Ok(bytes) => Ok(Some(bytes)),
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

    async fn put_bytes(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()> {
        check_key(key)?;
        if content_type.is_empty()
            || content_type.len() > 255
            || content_type.chars().any(char::is_control)
        {
            return Err(AppError::invalid_request("Invalid content type."));
        }
        if bytes.len() as u64 > self.max_bytes {
            return Err(AppError::limit_exceeded(format!(
                "Upload exceeds the maximum of {} bytes.",
                self.max_bytes
            ))
            .with_details(serde_json::json!({ "maxBytes": self.max_bytes })));
        }
        if !magic_matches(&essence(content_type), &bytes[..bytes.len().min(16)]) {
            return Err(AppError::invalid_request(
                "The file content does not match the declared content type.",
            ));
        }
        let (data_path, meta_path) = self.paths(key);
        for p in [&data_path, &meta_path] {
            if let Some(parent) = p.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(io_err)?;
            }
        }
        let tmp = self.dir.join(format!("{}.tmp", Uuid::now_v7().simple()));
        if let Err(e) = tokio::fs::write(&tmp, &bytes).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io_err(e));
        }
        tokio::fs::rename(&tmp, &data_path).await.map_err(io_err)?;
        let meta = Meta {
            key: key.to_owned(),
            content_type: content_type.to_owned(),
            size: bytes.len() as u64,
            // Direktschreiben (Import) kennt keinen ursprünglichen Dateinamen → Schlüssel-Fallback.
            filename: None,
        };
        tokio::fs::write(&meta_path, serde_json::to_vec(&meta).unwrap_or_default())
            .await
            .map_err(io_err)
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
                fname: None,
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
                fname: None,
            },
        ))
    }

    async fn signed_put_url_named(
        &self,
        key: &str,
        content_type: &str,
        max_size: u64,
        filename: &str,
        ttl: Duration,
    ) -> AppResult<String> {
        check_key(key)?;
        // Nur nicht-leere Namen mitsignieren; ein leerer Name führt beim Download ohnehin auf den
        // Schlüssel-Fallback.
        let fname = (!filename.trim().is_empty()).then(|| filename.to_owned());
        Ok(self.url(
            "uploads",
            &Claims {
                op: "put".into(),
                key: key.into(),
                ct: Some(content_type.into()),
                exp: self.expiry(ttl),
                max: Some(max_size),
                fname,
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
                fname: None,
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
    // web N1 (LIN-64): PUT-Token-Einmaligkeit. hard_link ist atomar und scheitert, wenn die
    // Zieldatei schon existiert — so kann derselbe (noch gültige) PUT-Token die Datei nicht ein
    // zweites Mal (ggf. groesser, nach dem Finalisieren) ueberschreiben und die Groessen-/Quota-
    // Pruefung umgehen. rename wuerde dagegen ueberschreiben.
    match tokio::fs::hard_link(&tmp, &data_path).await {
        Ok(()) => {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(AppError::conflict("This upload URL has already been used."));
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io_err(e));
        }
    }
    let meta = Meta {
        key: claims.key.clone(),
        content_type: claims.ct.clone().unwrap_or_default(),
        size,
        // LIN-85: ursprünglichen Dateinamen aus dem signierten PUT-Token an der Datei hinterlegen.
        filename: claims.fname.clone(),
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

/// CSP für HTML-Präsentationen auf der eigenen, cookielosen Datei-Origin (Weg B,
/// `docs/superpowers/specs/2026-11-13-html-anhang-tab.md`). `sandbox allow-scripts` ohne
/// `allow-same-origin` → opake Origin: das Folien-Skript läuft, erreicht aber weder App-Cookie
/// noch App-API noch Storage. `default-src 'none'` (kein `connect-src`) verbietet fetch/XHR/WS →
/// kein Nach-Hause-Telefonieren. Inline-Skript/-Style erlaubt (Decks sind „alles embedded");
/// Google-Fonts als einzige externe Quelle; Bilder nur als `data:`-URI.
const PRESENTATION_CSP: &str = "default-src 'none'; sandbox allow-scripts; \
script-src 'unsafe-inline'; style-src 'unsafe-inline' https://fonts.googleapis.com; \
font-src https://fonts.gstatic.com; img-src data:; base-uri 'none'; form-action 'none'; \
frame-ancestors 'none'";

/// Strikte CSP für alle anderen Datei-Antworten: keinerlei aktive Inhalte.
const STRICT_BLOB_CSP: &str = "default-src 'none'; sandbox";

/// Host (klein geschrieben, ohne Port) aus einer Basis-URL oder einem `Host`-Header.
pub(crate) fn host_only(s: &str) -> String {
    let after_scheme = s.split_once("://").map_or(s, |(_, rest)| rest);
    let host = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    host.trim().to_ascii_lowercase()
}

/// Host der Anfrage (klein, ohne Port) aus dem `Host`-Header.
pub(crate) fn request_host(headers: &HeaderMap) -> Option<String> {
    headers.get(HOST)?.to_str().ok().map(host_only)
}

/// Entscheidet `Content-Disposition`-Art und CSP für eine Datei-Antwort.
/// `files_origin` = die Anfrage traf exakt die konfigurierte, cookielose Datei-Origin.
/// Nur dort und nur für `text/html` wird inline + script-fähig ausgeliefert; sonst unverändert
/// (Download/Inline nach `inline_safe`, immer strikte CSP ohne aktive Inhalte).
fn presentation_delivery(content_type: &str, files_origin: bool) -> (&'static str, &'static str) {
    if files_origin && essence(content_type) == "text/html" {
        return ("inline", PRESENTATION_CSP);
    }
    let disposition = if inline_safe(content_type) {
        "inline"
    } else {
        "attachment"
    };
    (disposition, STRICT_BLOB_CSP)
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

/// Dateiendung zum Content-Type (für Downloads ohne Endung im Speicher-Key, LIN-23):
/// Clients wie Messenger entscheiden über die Vorschau oft anhand der Endung.
fn extension_for(content_type: &str) -> Option<&'static str> {
    Some(match essence(content_type).as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "text/csv" => "csv",
        "text/html" => "html",
        "application/json" => "json",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/rtf" => "rtf",
        // Office Open XML (Word/Excel/PowerPoint).
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        // Legacy Microsoft Office.
        "application/msword" => "doc",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.ms-powerpoint" => "ppt",
        // OpenDocument.
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        _ => return None,
    })
}

/// Anzeigename für `Content-Disposition` (LIN-85): bevorzugt den ursprünglichen Upload-Namen
/// (unicode-erhaltend, nur entschärft), sonst den letzten Key-Teil (ASCII, UUID). In beiden Fällen
/// wird eine Endung aus dem Content-Type ergänzt, wenn der Name keine hat.
fn download_filename(original: Option<&str>, key: &str, content_type: &str) -> String {
    let name = match original {
        Some(f) if !f.trim().is_empty() => sanitize_original(f),
        _ => filename_of(key),
    };
    match extension_for(content_type) {
        Some(ext) if !name.contains('.') => format!("{name}.{ext}"),
        _ => name,
    }
}

/// Entschärft den ursprünglichen Dateinamen für die Header-Verwendung: Steuerzeichen und
/// unsichtbare Bidi-/Format-Zeichen entfernen, Pfadtrenner neutralisieren – aber sichtbares
/// Unicode (CJK/Emoji) erhalten, das RFC 5987 kodiert.
///
/// Die Bidi-/Format-Zeichen (Kategorie Cf) würden `is_control()` passieren und im bevorzugten
/// `filename*`-Wert überleben; beim Anzeigen im Browser ermöglichen sie Right-to-Left-Override-
/// Spoofing (z. B. `report<U+202E>cod.exe` erscheint als `report.exe...`). `std` kennt keine
/// Unicode-Kategorien, darum die betroffenen Bereiche explizit (U+200B–200F, U+202A–202E,
/// U+2066–2069, U+FEFF).
fn sanitize_original(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !is_disallowed_name_char(*c))
        .map(|c| if matches!(c, '/' | '\\') { '_' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "file".into()
    } else {
        clean.to_owned()
    }
}

/// Steuerzeichen oder unsichtbares Bidi-/Format-Zeichen (Spoofing-Gefahr im angezeigten Namen).
fn is_disallowed_name_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069 | 0xFEFF
        )
}

/// Prozent-Kodierung nach RFC 5987 (`ext-value`): nur `attr-char` bleiben stehen, alles andere wird
/// als UTF-8-Bytes `%XX` kodiert. Für den `filename*`-Parameter nicht-ASCII-tauglicher Namen.
fn rfc5987_encode(s: &str) -> String {
    use std::fmt::Write;
    s.bytes()
        .fold(String::with_capacity(s.len()), |mut out, b| {
            match b {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'!'
                | b'#'
                | b'$'
                | b'&'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~' => out.push(b as char),
                _ => {
                    let _ = write!(out, "%{b:02X}");
                }
            }
            out
        })
}

/// Baut den `Content-Disposition`-Wert. Immer ein ASCII-sicherer `filename="…"` (sonst verwirft
/// `HeaderValue::from_str` den gesamten Header und der Download verlöre `attachment`); weicht der
/// ASCII-Fallback vom echten Namen ab (Nicht-ASCII, `"`, `\`), zusätzlich `filename*=UTF-8''…`
/// nach RFC 5987, das moderne Browser bevorzugen.
fn content_disposition(kind: &str, name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ascii = if ascii.trim().is_empty() {
        "file".to_owned()
    } else {
        ascii
    };
    if ascii == name {
        format!("{kind}; filename=\"{ascii}\"")
    } else {
        format!(
            "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{}",
            rfc5987_encode(name)
        )
    }
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
    headers: HeaderMap,
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
    // Weg B: Nur auf der konfigurierten, cookielosen Datei-Origin (exakter Host-Vergleich,
    // fail-closed) darf HTML inline + script-fähig laufen. Auf der App-Origin bleibt es Download.
    let files_origin = app
        .config
        .files_base_url
        .as_deref()
        .map(host_only)
        .zip(request_host(&headers))
        .is_some_and(|(want, got)| want == got);
    let (disposition_kind, csp) = presentation_delivery(&meta.content_type, files_origin);
    // web N4 (LIN-64): Nicht inline darstellbare Typen werden als Download (`attachment`)
    // ausgeliefert. Dafuer den Content-Type auf application/octet-stream zwingen, damit
    // hochgeladenes HTML/JS/SVG selbst bei einer kuenftigen HTML-Injection nicht als CSP-konformes
    // Skript interpretiert werden kann (das globale `nosniff` greift dann), und per
    // Cross-Origin-Resource-Policy: same-origin nicht cross-origin eingebettet wird. Inline-Typen
    // (Bilder/PDF/Video) und die HTML-Praesentation (Weg B) behalten ihren echten Typ, damit die
    // App sie wie vorgesehen cross-origin anzeigen bzw. einbetten kann.
    let is_attachment = disposition_kind == "attachment";
    let served_ct: &str = if is_attachment {
        "application/octet-stream"
    } else {
        &meta.content_type
    };
    let disposition = content_disposition(
        disposition_kind,
        &download_filename(meta.filename.as_deref(), &meta.key, &meta.content_type),
    );
    let mut res = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let h = res.headers_mut();
    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(served_ct)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if is_attachment {
        h.insert(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        );
    }
    h.insert(CONTENT_LENGTH, HeaderValue::from(meta.size));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(CONTENT_DISPOSITION, v);
    }
    // Nutzerinhalt führt nur auf der eigenen Datei-Origin (opake Sandbox) Skripte aus, nie im
    // Ursprung der App.
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
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
    async fn put_bytes_writes_directly_and_enforces_size_and_magic_bytes() {
        let dir = std::env::temp_dir().join(format!("lineal-putbytes-{}", Uuid::now_v7()));
        let s = LocalBlobStore::new(
            &dir,
            "change-me-32-bytes-minimum-xxxxxxxx",
            "http://unreachable.invalid",
            Arc::new(At(1_000)),
        )
        .with_max_bytes(32);
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        s.put_bytes("org/a/b", "image/png", png.clone())
            .await
            .unwrap();
        let info = s.head("org/a/b").await.unwrap().unwrap();
        assert_eq!(
            (info.size, info.content_type.as_str()),
            (png.len() as u64, "image/png")
        );
        assert_eq!(s.read("org/a/b", 100).await.unwrap().unwrap(), png);
        // Typ passt nicht zum Inhalt
        assert!(
            s.put_bytes("org/a/c", "image/png", b"not a png".to_vec())
                .await
                .is_err()
        );
        assert!(s.head("org/a/c").await.unwrap().is_none());
        // zu groß
        let err = s
            .put_bytes("org/a/d", "application/octet-stream", vec![0; 33])
            .await
            .unwrap_err();
        assert_eq!(err.kind, lineal_app::ErrorKind::LimitExceeded);
        // ungültiger Schlüssel/Typ
        assert!(s.put_bytes("", "image/png", png.clone()).await.is_err());
        assert!(s.put_bytes("org/a/e", "", png).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
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
                max: None,
                fname: None,
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
    fn html_renders_inline_only_on_the_files_origin() {
        // Review-Focus 1: App-Origin -> HTML bleibt Download + strikte Sandbox (kein Skript).
        let (disp, csp) = presentation_delivery("text/html", false);
        assert_eq!(disp, "attachment");
        assert_eq!(csp, STRICT_BLOB_CSP);
        // Review-Focus 2: Datei-Origin -> HTML inline + script-fähige Präsentations-CSP.
        let (disp, csp) = presentation_delivery("text/html; charset=utf-8", true);
        assert_eq!(disp, "inline");
        assert!(csp.contains("sandbox allow-scripts"));
        assert!(csp.contains("script-src 'unsafe-inline'"));
        assert!(csp.contains("font-src https://fonts.gstatic.com"));
        assert!(!csp.contains("connect-src"), "kein Nach-Hause-Telefonieren");
        assert!(
            !csp.contains("allow-same-origin"),
            "opake Origin bleibt opak"
        );
        // Review-Focus 3: Datei-Origin + Nicht-HTML -> unverändert.
        assert_eq!(
            presentation_delivery("image/png", true),
            ("inline", STRICT_BLOB_CSP)
        );
        assert_eq!(
            presentation_delivery("application/zip", true),
            ("attachment", STRICT_BLOB_CSP)
        );
        // SVG bleibt in dieser Stufe Download, auch auf der Datei-Origin.
        assert_eq!(
            presentation_delivery("image/svg+xml", true),
            ("attachment", STRICT_BLOB_CSP)
        );
    }

    #[test]
    fn host_match_is_exact_case_insensitive_and_port_stripped() {
        assert_eq!(
            host_only("https://Lineal-Files.Example/"),
            "lineal-files.example"
        );
        assert_eq!(
            host_only("lineal-files.example:443"),
            "lineal-files.example"
        );
        assert_eq!(
            host_only("HTTPS://lineal-files.example:8080/files/x"),
            "lineal-files.example"
        );
        // Review-Focus 4: Spoofing-Hosts matchen die Datei-Origin nicht.
        let want = host_only("https://lineal-files.example");
        assert_ne!(host_only("evil.lineal-files.example"), want);
        assert_ne!(host_only("lineal-files.example.evil.com"), want);
        // request_host liest + normalisiert den Header; fehlt er, None (fail-closed).
        let mut h = HeaderMap::new();
        h.insert(HOST, HeaderValue::from_static("Lineal-Files.Example:8080"));
        assert_eq!(request_host(&h).as_deref(), Some("lineal-files.example"));
        assert_eq!(request_host(&HeaderMap::new()), None);
    }

    #[test]
    fn keys_never_reach_the_file_system_directly() {
        let s = store(0);
        let (data, meta) = s.paths("../../etc/passwd");
        assert!(data.starts_with("/nonexistent/data"));
        assert!(!data.to_string_lossy().contains(".."));
        assert!(meta.to_string_lossy().ends_with(".json"));
        assert_eq!(filename_of("a/b/../ev il.html"), "ev_il.html");
        assert_eq!(
            download_filename(None, "org/x/01a1-uuid", "image/png"),
            "01a1-uuid.png"
        );
        assert_eq!(
            download_filename(None, "org/x/01a1-uuid", "IMAGE/JPEG; q=1"),
            "01a1-uuid.jpg"
        );
        assert_eq!(
            download_filename(None, "org/x/report.pdf", "image/png"),
            "report.pdf"
        );
        assert_eq!(
            download_filename(None, "org/x/blob", "application/x-unknown"),
            "blob"
        );
        // LIN-85: Office-Typen bekommen eine Endung, damit die Datei öffenbar ist (statt nackter UUID).
        assert_eq!(
            download_filename(
                None,
                "org/x/01a1-uuid",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            ),
            "01a1-uuid.pptx"
        );
        assert_eq!(
            download_filename(
                None,
                "org/x/01a1-uuid",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            "01a1-uuid.docx"
        );
        assert_eq!(
            download_filename(
                None,
                "org/x/01a1-uuid",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            ),
            "01a1-uuid.xlsx"
        );
        assert_eq!(
            download_filename(None, "org/x/01a1-uuid", "application/msword"),
            "01a1-uuid.doc"
        );
    }

    #[test]
    fn legacy_meta_without_filename_still_parses() {
        // LIN-85: Altbestand-Metadaten (vor dem neuen Feld) müssen weiter deserialisieren – sonst
        // fände `head`/`download` jede bestehende Datei als „korrupt"/fehlend.
        let meta: Meta =
            serde_json::from_str(r#"{"key":"org/x/uuid","content_type":"image/png","size":3}"#)
                .expect("legacy meta parses");
        assert_eq!(meta.filename, None);
        // Und der neue Serialize lässt das Feld weg, wenn es fehlt (kompakt, abwärtskompatibel).
        assert!(!serde_json::to_string(&meta).unwrap().contains("filename"));
    }

    #[test]
    fn download_filename_prefers_the_original_upload_name() {
        // LIN-85: ist der ursprüngliche Name hinterlegt, gewinnt er über die UUID im Key.
        assert_eq!(
            download_filename(
                Some("Fehleruebersicht.pptx"),
                "org/x/01a1-uuid",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            ),
            "Fehleruebersicht.pptx"
        );
        // Endung fehlt im Namen → aus dem Content-Type ergänzt.
        assert_eq!(
            download_filename(Some("report"), "org/x/01a1-uuid", "application/pdf"),
            "report.pdf"
        );
        // Unicode bleibt im Anzeigenamen erhalten (RFC-5987-Kodierung erst im Header).
        assert_eq!(
            download_filename(Some("月次報告.xlsx"), "org/x/uuid", "text/plain"),
            "月次報告.xlsx"
        );
        // Pfadtrenner werden neutralisiert, der Name nicht auf den Key zurückgesetzt.
        assert_eq!(
            download_filename(Some("a/b\\c.txt"), "org/x/uuid", "text/plain"),
            "a_b_c.txt"
        );
        // Leer/Whitespace → Schlüssel-Fallback (UUID + Endung).
        assert_eq!(
            download_filename(Some("   "), "org/x/uuid", "image/png"),
            "uuid.png"
        );
    }

    #[test]
    fn original_name_strips_bidi_override_spoofing() {
        // U+202E (Right-to-Left Override) würde `report<U+202E>cod.exe` als `report.exe...`
        // anzeigen; es wird aus dem Namen entfernt, bevor es in den `filename*`-Wert gelangt.
        assert_eq!(
            download_filename(Some("report\u{202E}cod.exe"), "org/x/uuid", "text/plain"),
            "reportcod.exe"
        );
        // Weitere unsichtbare Format-/Bidi-Zeichen fallen ebenso weg; sichtbares CJK bleibt.
        assert_eq!(
            sanitize_original("\u{FEFF}月\u{200F}次\u{2066}.txt"),
            "月次.txt"
        );
        // Und der erzeugte Header trägt keine der gefährlichen Codepoints mehr.
        let disp = content_disposition(
            "attachment",
            &download_filename(Some("a\u{202E}b.txt"), "org/x/uuid", "text/plain"),
        );
        assert!(
            !disp.contains("%E2%80%AE"),
            "bidi override must be gone: {disp}"
        );
    }

    #[test]
    fn content_disposition_is_ascii_safe_and_rfc5987_for_non_ascii() {
        // Reiner ASCII-Name → nur `filename="…"` (byte-identisch zum bisherigen Verhalten).
        assert_eq!(
            content_disposition("attachment", "Fehleruebersicht.pptx"),
            "attachment; filename=\"Fehleruebersicht.pptx\""
        );
        // Leerzeichen sind im quoted-string erlaubt, kein `filename*` nötig.
        assert_eq!(
            content_disposition("inline", "my report.pdf"),
            "inline; filename=\"my report.pdf\""
        );
        // CJK → ASCII-Fallback (Platzhalter) plus RFC-5987-`filename*`.
        assert_eq!(
            content_disposition("attachment", "月次.xlsx"),
            "attachment; filename=\"__.xlsx\"; filename*=UTF-8''%E6%9C%88%E6%AC%A1.xlsx"
        );
        // Emoji ebenso.
        assert_eq!(
            content_disposition("attachment", "🎉.png"),
            "attachment; filename=\"_.png\"; filename*=UTF-8''%F0%9F%8E%89.png"
        );
        // Anführungszeichen/Backslash im Namen werden im Fallback ersetzt und per `filename*`
        // sicher transportiert – der Header bleibt wohlgeformt.
        let got = content_disposition("attachment", "a\"b\\c.txt");
        assert_eq!(
            got,
            "attachment; filename=\"a_b_c.txt\"; filename*=UTF-8''a%22b%5Cc.txt"
        );
        // Jeder erzeugte Wert ist als Header-Value akzeptabel (ASCII, keine Steuerzeichen) – sonst
        // verwürfe `HeaderValue::from_str` den Header und der Download verlöre `attachment`.
        for name in ["a\"b\\c.txt", "月次.xlsx", "🎉.png", "x\u{7f}.bin"] {
            assert!(HeaderValue::from_str(&content_disposition("attachment", name)).is_ok());
        }
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
            max: None,
            fname: None,
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

    /// LIN-64 web N1: ein PUT-Token ist einmalig – ein zweiter PUT auf denselben Key
    /// ueberschreibt die Datei nicht (sonst liesse sich die Groessenpruefung umgehen).
    #[tokio::test]
    async fn upload_url_is_single_use() {
        let dir = std::env::temp_dir().join(format!("lineal-blob-once-{}", Uuid::now_v7()));
        let s = LocalBlobStore::new(
            &dir,
            "change-me-32-bytes-minimum-xxxxxxxx",
            "http://x",
            Arc::new(At(0)),
        );
        let claims = Claims {
            op: "put".into(),
            key: "org/once".into(),
            ct: Some("text/plain".into()),
            exp: 1,
            max: None,
            fname: None,
        };
        assert_eq!(
            write_upload(&s, &claims, Body::from(vec![1u8; 4]))
                .await
                .unwrap(),
            4
        );
        let err = write_upload(&s, &claims, Body::from(vec![2u8; 9000]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, lineal_app::ErrorKind::Conflict);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// LIN-64 web N4: nicht inline-sichere Typen werden als `attachment` ausgeliefert (download()
    /// sendet dafuer application/octet-stream + CORP: same-origin); inline-sichere Typen und die
    /// HTML-Praesentation auf der Datei-Origin behalten ihren echten Typ.
    #[test]
    fn presentation_decides_disposition_for_n4() {
        assert_eq!(presentation_delivery("image/png", false).0, "inline");
        assert_eq!(presentation_delivery("application/pdf", false).0, "inline");
        assert_eq!(presentation_delivery("text/html", false).0, "attachment");
        assert_eq!(
            presentation_delivery("image/svg+xml", false).0,
            "attachment"
        );
        assert_eq!(
            presentation_delivery("application/javascript", false).0,
            "attachment"
        );
        let (kind, csp) = presentation_delivery("text/html", true);
        assert_eq!(kind, "inline");
        assert_eq!(csp, PRESENTATION_CSP);
    }
}
