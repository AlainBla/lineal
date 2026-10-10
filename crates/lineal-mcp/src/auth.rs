//! `auth` (WP04): Bearer-Auth (API-Key/OAuth-Token) → `RequestCtx`, Scope-Prüfung.
//!
//! Token-Hash-Konvention (muss mit WP02/WP16 übereinstimmen): **SHA-256 über die rohen Bytes des
//! vollständigen Tokens (inkl. Präfix `lin_api_…`), kleingeschrieben hexadezimal** (64 Zeichen).
//! Die Auflösung macht `AuthRepo::find_principal_by_token_hash`.

use crate::wellknown::protected_resource_metadata_url;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::json;

/// SHA-256-Hex (klein) eines Tokens – Schlüssel für `find_principal_by_token_hash`.
pub use lineal_app::usecases::auth::hash_token;

/// Neue Request-ID im Format `req_<uuid-v7 ohne Bindestriche>`.
pub fn new_request_id() -> String {
    format!("req_{}", uuid::Uuid::now_v7().simple())
}

/// Liest das Token aus `Authorization: Bearer <token>` (Schema case-insensitiv).
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// Warum die Authentifizierung scheiterte.
#[derive(Debug)]
pub enum AuthFailure {
    /// Kein `Authorization: Bearer`-Header.
    Missing,
    /// Token unbekannt, widerrufen oder abgelaufen.
    Invalid,
    /// Fehler beim Nachschlagen (Datenbank).
    Internal(AppError),
}

/// Löst das Bearer-Token auf und baut den `RequestCtx` (Org, Actor, Scopes, neue Request-ID).
pub async fn authenticate(app: &App, headers: &HeaderMap) -> Result<RequestCtx, AuthFailure> {
    let token = bearer_token(headers).ok_or(AuthFailure::Missing)?;
    let hash = hash_token(token);
    let mut tx = app
        .store
        .begin_system()
        .await
        .map_err(AuthFailure::Internal)?;
    let principal = tx
        .auth()
        .find_principal_by_token_hash(&hash)
        .await
        .map_err(AuthFailure::Internal)?
        .ok_or(AuthFailure::Invalid)?;
    Ok(RequestCtx {
        org_id: principal.org_id,
        actor: principal.actor,
        request_id: new_request_id(),
        scopes: principal.scopes,
        team_scope: principal.team_scope,
    })
}

/// HTTP-Antwort für eine gescheiterte Authentifizierung: `401` mit
/// `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource"`
/// (docs/07 §1.3) und JSON-Fehlermodell im Body; Datenbankfehler → `500`.
pub fn failure_response(app: &App, failure: AuthFailure) -> Response {
    let request_id = new_request_id();
    let (err, error_param) = match failure {
        AuthFailure::Missing => (
            AppError::unauthenticated("Missing Bearer token. Send Authorization: Bearer <token>"),
            None,
        ),
        AuthFailure::Invalid => (
            AppError::unauthenticated("Invalid or expired token"),
            Some("invalid_token"),
        ),
        AuthFailure::Internal(e) => {
            tracing::error!(error = %e, "token lookup failed");
            let body = crate::error::error_body(&AppError::internal("Internal error"), &request_id);
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };
    let resource_metadata = protected_resource_metadata_url(app);
    let challenge = match error_param {
        Some(e) => format!(r#"Bearer error="{e}", resource_metadata="{resource_metadata}""#),
        None => format!(r#"Bearer resource_metadata="{resource_metadata}""#),
    };
    let mut response = (
        StatusCode::UNAUTHORIZED,
        axum::Json(crate::error::error_body(&err, &request_id)),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    response
}

/// Benötigter Scope eines Tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeNeed {
    Read,
    Write,
}

/// `read` für `list_*`, `get_*`, `extract_images`, `search_documentation`; alles andere `write`
/// (docs/07 §1.3).
pub fn required_scope(tool: &str) -> ScopeNeed {
    if tool.starts_with("list_")
        || tool.starts_with("get_")
        || tool == "extract_images"
        || tool == "search_documentation"
    {
        ScopeNeed::Read
    } else {
        ScopeNeed::Write
    }
}

/// `write` impliziert `read`.
pub fn scope_allows(scopes: lineal_app::Scopes, need: ScopeNeed) -> bool {
    match need {
        ScopeNeed::Read => scopes.read || scopes.write,
        ScopeNeed::Write => scopes.write,
    }
}

/// Prüft den Scope für `tool`; sonst `forbidden`.
pub fn ensure_scope(ctx: &RequestCtx, tool: &str) -> AppResult<()> {
    let need = required_scope(tool);
    if scope_allows(ctx.scopes, need) {
        Ok(())
    } else {
        let scope = if need == ScopeNeed::Write {
            "write"
        } else {
            "read"
        };
        Err(
            AppError::forbidden(format!("Tool \"{tool}\" requires the \"{scope}\" scope"))
                .with_details(json!({ "requiredScope": scope })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::Scopes;

    #[test]
    fn sha256_hex_is_lowercase_64_chars() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn bearer_parsing() {
        let mut h = HeaderMap::new();
        assert_eq!(bearer_token(&h), None);
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer tok"),
        );
        assert_eq!(bearer_token(&h), Some("tok"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer_token(&h), None);
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert_eq!(bearer_token(&h), None);
    }

    #[test]
    fn scope_classification() {
        for t in [
            "list_issues",
            "get_issue",
            "extract_images",
            "search_documentation",
        ] {
            assert_eq!(required_scope(t), ScopeNeed::Read, "{t}");
        }
        for t in [
            "save_issue",
            "delete_comment",
            "retire_issue_label",
            "restore_issue_label",
            "mark_notification",
            "share_issue",
            "unshare_issue",
            "prepare_attachment_upload",
            "create_attachment_from_upload",
        ] {
            assert_eq!(required_scope(t), ScopeNeed::Write, "{t}");
        }
        assert!(scope_allows(Scopes::read_only(), ScopeNeed::Read));
        assert!(!scope_allows(Scopes::read_only(), ScopeNeed::Write));
        let write_only = Scopes {
            read: false,
            write: true,
            admin: false,
        };
        assert!(scope_allows(write_only, ScopeNeed::Read));
    }
}
