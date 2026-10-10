//! Security-Header (docs/10 §11.4): strenge CSP ohne Inline-Skripte, HSTS (nur hinter HTTPS),
//! `X-Content-Type-Options`, `Referrer-Policy`, Clickjacking-Schutz.

use axum::extract::{Request, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// CSP der Web-App: nur eigene Skripte; Styles inline erlaubt (Tailwind/Radix setzen
/// `style`-Attribute); Verbindungen inkl. WebSocket (`/sync`) nur zum eigenen Ursprung.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' ws: wss:; \
object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// Einstellungen der Header-Middleware.
#[derive(Debug, Clone, Copy)]
pub struct SecurityConfig {
    /// HSTS senden (nur sinnvoll, wenn der Server über HTTPS erreichbar ist).
    pub hsts: bool,
    /// Dev: GraphiQL (`GET /graphql`) lädt Inline-Skripte/CDN und bekommt keine CSP.
    pub dev: bool,
}

/// Setzt die Header auf die Antwort; bereits gesetzte Header (z. B. strengere CSP des
/// Blob-Endpunkts) bleiben unverändert.
pub fn apply_headers(headers: &mut HeaderMap, cfg: &SecurityConfig, path: &str) {
    let skip_csp = cfg.dev && path == "/graphql";
    if !skip_csp && !headers.contains_key(CONTENT_SECURITY_POLICY) {
        headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    }
    headers
        .entry(X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("strict-origin-when-cross-origin"));
    headers
        .entry(X_FRAME_OPTIONS)
        .or_insert(HeaderValue::from_static("DENY"));
    if cfg.hsts {
        headers
            .entry(STRICT_TRANSPORT_SECURITY)
            .or_insert(HeaderValue::from_static(
                "max-age=31536000; includeSubDomains",
            ));
    }
}

pub async fn middleware(State(cfg): State<SecurityConfig>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let mut res = next.run(req).await;
    apply_headers(res.headers_mut(), &cfg, &path);
    res
}
