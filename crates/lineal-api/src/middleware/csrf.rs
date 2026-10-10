//! CSRF-Schutz für Cookie-Sessions (docs/15 §Sicherheit).
//!
//! Browser hängen das Session-Cookie an jede Anfrage an, auch an fremd ausgelöste. Deshalb
//! gilt für zustandsändernde Anfragen, die **nur** per Cookie authentifiziert sind:
//!
//! * `POST /graphql` und `POST /auth/logout`: `Origin` (ersatzweise `Referer`) muss in der
//!   Allow-List stehen **und** der Header `X-Lineal-CSRF: 1` gesetzt sein (ein fremder Origin
//!   kann ihn ohne CORS-Preflight nicht senden).
//! * `POST /auth/login`, `POST /invite/accept`, `POST /oauth/authorize` (HTML-Formulare ohne eigene Header; der
//!   session-gebundene Nonce ist dort das Token) und `PUT /uploads/*` (signierte URL): nur
//!   Origin-Prüfung. Fehlt der Origin ganz (kein Browser-Cross-Site-Request), wird nicht
//!   abgelehnt; ein vorhandener fremder Origin immer.
//!
//! Ausgenommen sind Anfragen mit `Authorization`-Header (API-Key/OAuth, kein ambient
//! credential) sowie `/mcp` und `/sync` (eigene Prüfung).

use crate::error::ApiError;
use crate::middleware::Platform;
use crate::middleware::auth::{CredentialClass, extract_token};
use axum::extract::{Request, State};
use axum::http::header::{ORIGIN, REFERER};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lineal_app::AppError;

/// Name des Pflicht-Headers für Cookie-Sessions auf `/graphql` und `/auth/logout`.
pub const CSRF_HEADER: &str = "x-lineal-csrf";

/// Wie streng eine Anfrage geprüft wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    None,
    /// Origin vorhanden und erlaubt; fehlender Origin wird toleriert.
    OriginIfPresent,
    /// Origin Pflicht und erlaubt, plus `X-Lineal-CSRF: 1`.
    OriginAndHeader,
}

/// Erlaubte Origins: Konfiguration oder Origin der Basis-URL; im Dev-Modus zusätzlich lokale Hosts.
pub fn effective_origins(configured: &[String], base_url: &str) -> Vec<String> {
    if configured.is_empty() {
        origin_of(base_url).into_iter().collect()
    } else {
        configured
            .iter()
            .map(|o| o.trim_end_matches('/').to_ascii_lowercase())
            .collect()
    }
}

/// `scheme://host[:port]` einer URL (ohne Pfad), kleingeschrieben.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if scheme.is_empty() || host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{host}").to_ascii_lowercase())
}

fn is_local_dev_origin(origin: &str) -> bool {
    ["http://localhost", "http://127.0.0.1"].iter().any(|base| {
        origin
            .strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.strip_prefix(':').is_some_and(is_port))
    })
}

fn is_port(s: &str) -> bool {
    !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Ob `origin` erlaubt ist.
pub fn origin_allowed(origin: &str, allowed: &[String], dev: bool) -> bool {
    let origin = origin.trim_end_matches('/').to_ascii_lowercase();
    allowed.contains(&origin) || (dev && is_local_dev_origin(&origin))
}

/// Origin der Anfrage: `Origin`-Header, sonst Origin des `Referer`. `None` = beides fehlt;
/// `Some("null")` und Unlesbares sind nie erlaubt.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(ORIGIN) {
        return Some(v.to_str().unwrap_or("null").to_owned());
    }
    let referer = headers.get(REFERER)?;
    Some(
        referer
            .to_str()
            .ok()
            .and_then(origin_of)
            .unwrap_or_else(|| "null".to_owned()),
    )
}

fn rule_for(method: &Method, path: &str, has_cookie_session: bool) -> Rule {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Rule::None;
    }
    match path {
        "/graphql" | "/auth/logout" if has_cookie_session => Rule::OriginAndHeader,
        "/auth/login" | "/invite/accept" => Rule::OriginIfPresent,
        "/oauth/authorize" if has_cookie_session => Rule::OriginIfPresent,
        p if p.starts_with("/uploads/") => Rule::OriginIfPresent,
        _ => Rule::None,
    }
}

/// Prüft eine Anfrage; `Err` = 403.
pub fn check(
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    allowed: &[String],
    dev: bool,
) -> Result<(), AppError> {
    // `/mcp` und `/sync` haben eigene Prüfungen; Authorization-Header = kein Cookie-Request.
    if path.starts_with("/mcp") || path.starts_with("/sync") {
        return Ok(());
    }
    let has_cookie_session = matches!(extract_token(headers), Some((_, CredentialClass::Session)));
    let rule = rule_for(method, path, has_cookie_session);
    if rule == Rule::None {
        return Ok(());
    }
    match request_origin(headers) {
        Some(origin) if !origin_allowed(&origin, allowed, dev) => {
            return Err(AppError::forbidden("Cross-origin request rejected."));
        }
        None if rule == Rule::OriginAndHeader => {
            return Err(AppError::forbidden("Missing Origin header."));
        }
        _ => {}
    }
    if rule == Rule::OriginAndHeader
        && headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()) != Some("1")
    {
        return Err(AppError::forbidden("Missing X-Lineal-CSRF header."));
    }
    Ok(())
}

pub async fn middleware(State(p): State<Platform>, req: Request, next: Next) -> Response {
    if let Err(e) = check(
        req.method(),
        req.uri().path(),
        req.headers(),
        &p.cfg.allowed_origins,
        p.cfg.security.dev,
    ) {
        return ApiError(e).into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn allowed() -> Vec<String> {
        vec!["https://lineal.example".into()]
    }

    const COOKIE: (&str, &str) = ("cookie", "lineal_session=tok");

    #[test]
    fn graphql_with_cookie_needs_origin_and_header() {
        let ok = headers(&[
            COOKIE,
            ("origin", "https://lineal.example"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &ok, &allowed(), false).is_ok());
        let cross = headers(&[
            COOKIE,
            ("origin", "https://evil.example"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &cross, &allowed(), false).is_err());
        let no_header = headers(&[COOKIE, ("origin", "https://lineal.example")]);
        assert!(check(&Method::POST, "/graphql", &no_header, &allowed(), false).is_err());
        let no_origin = headers(&[COOKIE, ("x-lineal-csrf", "1")]);
        assert!(check(&Method::POST, "/graphql", &no_origin, &allowed(), false).is_err());
        assert!(check(&Method::POST, "/auth/logout", &no_header, &allowed(), false).is_err());
    }

    #[test]
    fn referer_is_a_fallback_for_origin() {
        let h = headers(&[
            COOKIE,
            ("referer", "https://lineal.example/acme/inbox"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &h, &allowed(), false).is_ok());
        let bad = headers(&[
            COOKIE,
            ("referer", "https://evil.example/x"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &bad, &allowed(), false).is_err());
    }

    #[test]
    fn authorization_header_mcp_and_sync_are_exempt() {
        let key = headers(&[("authorization", "lin_api_x"), COOKIE]);
        assert!(check(&Method::POST, "/graphql", &key, &allowed(), false).is_ok());
        let bare = headers(&[COOKIE]);
        assert!(check(&Method::POST, "/mcp", &bare, &allowed(), false).is_ok());
        assert!(check(&Method::POST, "/sync", &bare, &allowed(), false).is_ok());
        // Ohne Session (anonym) entscheidet die Authentifizierung, nicht CSRF.
        assert!(
            check(
                &Method::POST,
                "/graphql",
                &HeaderMap::new(),
                &allowed(),
                false
            )
            .is_ok()
        );
        assert!(check(&Method::GET, "/auth/me", &bare, &allowed(), false).is_ok());
    }

    #[test]
    fn forms_check_origin_only() {
        let cross = headers(&[COOKIE, ("origin", "https://evil.example")]);
        assert!(check(&Method::POST, "/oauth/authorize", &cross, &allowed(), false).is_err());
        assert!(check(&Method::POST, "/auth/login", &cross, &allowed(), false).is_err());
        assert!(check(&Method::PUT, "/uploads/tok", &cross, &allowed(), false).is_err());
        let same = headers(&[COOKIE, ("origin", "https://lineal.example")]);
        assert!(check(&Method::POST, "/oauth/authorize", &same, &allowed(), false).is_ok());
        // Ohne Origin (kein Browser-Cross-Site): Nonce bzw. Signatur schützen.
        let none = headers(&[COOKIE]);
        assert!(check(&Method::POST, "/oauth/authorize", &none, &allowed(), false).is_ok());
        assert!(check(&Method::POST, "/oauth/token", &cross, &allowed(), false).is_ok());
    }

    #[test]
    fn dev_allows_local_origins_only_in_dev() {
        let h = headers(&[
            COOKIE,
            ("origin", "http://localhost:5173"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &h, &allowed(), true).is_ok());
        assert!(check(&Method::POST, "/graphql", &h, &allowed(), false).is_err());
        let sneaky = headers(&[
            COOKIE,
            ("origin", "http://localhost.evil.example"),
            ("x-lineal-csrf", "1"),
        ]);
        assert!(check(&Method::POST, "/graphql", &sneaky, &allowed(), true).is_err());
        let null = headers(&[COOKIE, ("origin", "null"), ("x-lineal-csrf", "1")]);
        assert!(check(&Method::POST, "/graphql", &null, &allowed(), true).is_err());
    }

    #[test]
    fn default_origin_comes_from_base_url() {
        assert_eq!(
            effective_origins(&[], "https://Lineal.example/app"),
            vec!["https://lineal.example".to_owned()]
        );
        assert_eq!(
            effective_origins(&["https://a.example/".into()], "http://x"),
            vec!["https://a.example".to_owned()]
        );
    }
}
