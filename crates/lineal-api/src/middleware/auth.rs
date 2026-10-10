//! Auth-Extraktor (docs/09 §2): Session-Cookie `lineal_session=<token>` oder
//! `Authorization: <key>` / `Authorization: Bearer <token>` -> SHA-256 -> `AuthRepo` -> `RequestCtx`.
//!
//! Die Middleware lehnt selbst nichts ab (Login, Health, Blob-URLs sind öffentlich); sie legt
//! den [`AuthState`] in die Request-Extensions. Geschützte Handler verlangen [`Authenticated`].

use crate::error::ApiError;
use crate::middleware::Platform;
use crate::middleware::request_id::RequestId;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lineal_app::ports::repos::Principal;
use lineal_app::{App, AppError, AppResult, RequestCtx};

/// Name des Session-Cookies.
pub const SESSION_COOKIE: &str = "lineal_session";
/// Längere Tokens sind nie gültig (Schutz vor Hashing riesiger Header).
const MAX_TOKEN_LEN: usize = 512;

/// Art der Anmeldung; bestimmt die Rate-Limit-Klasse (09 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialClass {
    Anonymous,
    ApiKey,
    OAuth,
    Session,
}

/// Ergebnis der Authentifizierung dieser Anfrage.
#[derive(Debug, Clone)]
pub enum AuthState {
    /// Keine Zugangsdaten gesendet.
    Anonymous,
    /// Zugangsdaten gesendet, aber unbekannt/abgelaufen/widerrufen.
    Invalid,
    Authenticated {
        ctx: RequestCtx,
        class: CredentialClass,
    },
}

impl AuthState {
    /// `RequestCtx` oder `unauthenticated`.
    pub fn require(&self) -> AppResult<&RequestCtx> {
        match self {
            Self::Authenticated { ctx, .. } => Ok(ctx),
            Self::Anonymous => Err(AppError::unauthenticated("Authentication required.")),
            Self::Invalid => Err(AppError::unauthenticated("Invalid or expired token.")),
        }
    }
}

/// SHA-256 des Tokens als Hex (Format des Lookups, 09 §2.2).
pub use lineal_app::usecases::auth::hash_token;

/// Token aus der Anfrage: `Authorization` hat Vorrang vor dem Cookie.
pub fn extract_token(headers: &HeaderMap) -> Option<(String, CredentialClass)> {
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        let raw = match value.split_once(' ') {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => rest.trim(),
            _ => value,
        };
        if !raw.is_empty() {
            // Die Klasse (und damit das Rate-Limit) folgt ausschließlich dem Token-Präfix.
            // Das Präfix ist Teil des gehashten Tokens und damit nicht fälschbar; das Schema
            // (`Bearer`) bestimmt der Client und darf kein höheres Limit freischalten.
            let class = if raw.starts_with("lin_oauth_") {
                CredentialClass::OAuth
            } else {
                CredentialClass::ApiKey
            };
            return Some((raw.to_owned(), class));
        }
    }
    for header in headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for pair in header.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=')
                && name == SESSION_COOKIE
                && !value.is_empty()
            {
                return Some((value.to_owned(), CredentialClass::Session));
            }
        }
    }
    None
}

/// Schlägt das Token nach (mandantenübergreifende Transaktion, nur Lesen).
pub async fn lookup_principal(app: &App, token: &str) -> AppResult<Option<Principal>> {
    if token.len() > MAX_TOKEN_LEN {
        return Ok(None);
    }
    let mut tx = app.store.begin_system().await?;
    tx.auth()
        .find_principal_by_token_hash(&hash_token(token))
        .await
}

/// `Set-Cookie`-Wert für eine neue Session (docs/10 §11.4: HttpOnly; Secure; SameSite=Lax).
pub fn session_cookie(token: &str, max_age_secs: u64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}"
    )
}

/// `Set-Cookie`-Wert, der die Session löscht (Logout).
pub fn clear_session_cookie(secure: bool) -> String {
    session_cookie("", 0, secure)
}

pub async fn middleware(State(p): State<Platform>, mut req: Request, next: Next) -> Response {
    let now_ms = p.app.clock.now().timestamp_millis();
    let state = match extract_token(req.headers()) {
        None => AuthState::Anonymous,
        Some((token, class)) => {
            // Fehlversuche (unbekannte/abgelaufene Tokens) zählen gegen den IP-Bucket und werden
            // bei Erschöpfung abgelehnt, bevor die Datenbank gefragt wird.
            let fail_key =
                crate::rate_limit::client_bucket(&req).map(|ip| format!("authfail:{ip}"));
            let limits = p.cfg.rate.anonymous;
            if let Some(key) = &fail_key
                && !p
                    .limiter
                    .has_token(key, limits.requests, p.cfg.rate.period_secs, now_ms)
            {
                let d = p
                    .limiter
                    .take(key, limits.requests, p.cfg.rate.period_secs, 1.0, now_ms);
                return crate::rate_limit::rejection(&req, &d, "failed authentications");
            }
            match lookup_principal(&p.app, &token).await {
                Ok(Some(principal)) => {
                    let request_id = req
                        .extensions()
                        .get::<RequestId>()
                        .map(|r| r.0.clone())
                        .unwrap_or_default();
                    AuthState::Authenticated {
                        ctx: RequestCtx {
                            org_id: principal.org_id,
                            actor: principal.actor,
                            request_id,
                            scopes: principal.scopes,
                        },
                        class,
                    }
                }
                Ok(None) => {
                    if let Some(key) = &fail_key {
                        p.limiter
                            .take(key, limits.requests, p.cfg.rate.period_secs, 1.0, now_ms);
                    }
                    AuthState::Invalid
                }
                Err(e) => return ApiError(e).into_response(),
            }
        }
    };
    req.extensions_mut().insert(state);
    next.run(req).await
}

/// Extraktor für geschützte REST-Handler: liefert den `RequestCtx` oder antwortet 401.
#[derive(Debug, Clone)]
pub struct Authenticated(pub RequestCtx);

impl Authenticated {
    /// `forbidden`, wenn der Token keinen Schreib-Scope hat.
    pub fn require_write(&self) -> AppResult<()> {
        if self.0.scopes.write {
            Ok(())
        } else {
            Err(AppError::forbidden("Token has no write scope."))
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<AuthState>() {
            Some(state) => Ok(Self(state.require()?.clone())),
            None => Err(ApiError(AppError::unauthenticated(
                "Authentication required.",
            ))),
        }
    }
}

/// Infallibler Extraktor für Handler, die den Zustand selbst auswerten (GraphQL).
#[derive(Debug, Clone)]
pub struct MaybeAuth(pub AuthState);

impl<S: Send + Sync> FromRequestParts<S> for MaybeAuth {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<AuthState>()
                .cloned()
                .unwrap_or(AuthState::Anonymous),
        ))
    }
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

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn token_sources_and_classes() {
        let t = |h| extract_token(&h);
        assert_eq!(
            t(headers(&[("authorization", "lin_api_x")])),
            Some(("lin_api_x".into(), CredentialClass::ApiKey))
        );
        assert_eq!(
            t(headers(&[("authorization", "Bearer lin_oauth_y")])),
            Some(("lin_oauth_y".into(), CredentialClass::OAuth))
        );
        assert_eq!(
            t(headers(&[("authorization", "bearer lin_api_z")])),
            Some(("lin_api_z".into(), CredentialClass::ApiKey))
        );
        assert_eq!(
            t(headers(&[("cookie", "a=b; lineal_session=tok; c=d")])),
            Some(("tok".into(), CredentialClass::Session))
        );
        // Authorization schlägt Cookie
        assert_eq!(
            t(headers(&[
                ("cookie", "lineal_session=tok"),
                ("authorization", "Bearer k")
            ])),
            Some(("k".into(), CredentialClass::ApiKey))
        );
        // `Bearer` allein schaltet kein höheres (OAuth-)Limit frei.
        assert_eq!(
            t(headers(&[("authorization", "Bearer random-token")])),
            Some(("random-token".into(), CredentialClass::ApiKey))
        );
        assert_eq!(t(headers(&[])), None);
        assert_eq!(t(headers(&[("cookie", "lineal_session=")])), None);
    }

    #[test]
    fn cookie_attributes() {
        let c = session_cookie("tok", 60, true);
        assert!(c.contains("HttpOnly") && c.contains("Secure") && c.contains("SameSite=Lax"));
        assert!(clear_session_cookie(false).contains("Max-Age=0"));
    }
}
