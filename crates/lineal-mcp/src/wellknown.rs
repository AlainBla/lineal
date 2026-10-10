//! `/.well-known/oauth-protected-resource` (RFC 9728) und
//! `/.well-known/oauth-authorization-server` (RFC 8414), docs/07 §1.3. Nur Metadaten – die
//! Endpunkte `/oauth/*` liefert WP16. Basis-URL aus `AppConfig::base_url`.

use axum::Json;
use axum::extract::State;
use lineal_app::App;
use serde_json::{Value, json};
use std::sync::Arc;

fn base(app: &App) -> &str {
    app.config.base_url.trim_end_matches('/')
}

/// URL des Metadaten-Dokuments der Ressource (für `WWW-Authenticate`).
pub fn protected_resource_metadata_url(app: &App) -> String {
    format!("{}/.well-known/oauth-protected-resource", base(app))
}

/// Metadaten der Protected Resource `/mcp`.
pub fn protected_resource_metadata(app: &App) -> Value {
    let base = base(app);
    json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": ["read", "write"],
        "bearer_methods_supported": ["header"],
    })
}

/// Metadaten des Authorization Servers.
pub fn authorization_server_metadata(app: &App) -> Value {
    let base = base(app);
    json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["read", "write"],
    })
}

/// `GET /.well-known/oauth-protected-resource` (auch mit Pfadsuffix `/mcp`).
pub async fn protected_resource(State(app): State<Arc<App>>) -> Json<Value> {
    Json(protected_resource_metadata(&app))
}

/// `GET /.well-known/oauth-authorization-server`.
pub async fn authorization_server(State(app): State<Arc<App>>) -> Json<Value> {
    Json(authorization_server_metadata(&app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_testkit::fake_app;

    #[test]
    fn metadata_follows_base_url() {
        let app = fake_app();
        let pr = protected_resource_metadata(&app);
        assert_eq!(pr["resource"], "http://localhost:8080/mcp");
        assert_eq!(pr["authorization_servers"][0], "http://localhost:8080");
        let asm = authorization_server_metadata(&app);
        assert_eq!(asm["code_challenge_methods_supported"][0], "S256");
        assert_eq!(asm["token_endpoint"], "http://localhost:8080/oauth/token");
        assert_eq!(
            protected_resource_metadata_url(&app),
            "http://localhost:8080/.well-known/oauth-protected-resource"
        );
    }
}
