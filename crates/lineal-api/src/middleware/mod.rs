//! Axum-Middleware des API-Adapters (docs/11 §3.5): Request-ID/Tracing, Security-Header,
//! CORS (nur dev), Auth zu `RequestCtx`, Rate-Limit. Reihenfolge von außen nach innen.

pub mod auth;
pub mod gql;
pub mod request_id;

use crate::rate_limit::{self, RateLimiter, RateLimits};
use crate::security_headers::{self, SecurityConfig};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::{Extension, Router, middleware::from_fn, middleware::from_fn_with_state};
use lineal_app::App;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

/// Konfiguration der Plattform-Middleware.
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    pub rate: RateLimits,
    pub security: SecurityConfig,
    /// Dev: erlaubte Ursprünge des Vite-Dev-Servers. Leer = kein CORS.
    pub cors_origins: Vec<String>,
}

impl PlatformConfig {
    /// Defaults abhängig von `dev` und `base_url` (HSTS nur bei `https://`).
    pub fn new(app: &App, dev: bool) -> Self {
        Self {
            rate: RateLimits::default(),
            security: SecurityConfig {
                hsts: app.config.base_url.starts_with("https://"),
                dev,
            },
            cors_origins: if dev {
                vec![
                    "http://localhost:5173".into(),
                    "http://127.0.0.1:5173".into(),
                ]
            } else {
                vec![]
            },
        }
    }
}

/// Geteilter Zustand der Middleware.
#[derive(Clone)]
pub struct Platform {
    pub app: Arc<App>,
    pub limiter: Arc<RateLimiter>,
    pub cfg: Arc<PlatformConfig>,
}

fn cors(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let exposed = [
        "x-request-id",
        "retry-after",
        "x-complexity",
        "x-ratelimit-requests-limit",
        "x-ratelimit-requests-remaining",
        "x-ratelimit-requests-reset",
        "x-ratelimit-complexity-limit",
        "x-ratelimit-complexity-remaining",
        "x-ratelimit-complexity-reset",
    ]
    .map(HeaderName::from_static);
    CorsLayer::new()
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-request-id"),
        ])
        .expose_headers(exposed)
}

/// Umhüllt den Router mit der Plattform-Middleware (Standardkonfiguration).
pub fn apply(router: Router, app: Arc<App>, dev: bool) -> Router {
    let cfg = PlatformConfig::new(&app, dev);
    apply_with(router, app, cfg)
}

/// Wie [`apply`] mit eigener Konfiguration (Tests, abweichende Limits).
pub fn apply_with(router: Router, app: Arc<App>, cfg: PlatformConfig) -> Router {
    let platform = Platform {
        app: app.clone(),
        limiter: Arc::new(RateLimiter::default()),
        cfg: Arc::new(cfg),
    };
    let security = platform.cfg.security;
    let cors_origins = platform.cfg.cors_origins.clone();
    // `layer` umhüllt: zuletzt hinzugefügt = äußerste Schicht.
    let mut router = router
        .layer(Extension(app))
        .layer(from_fn_with_state(platform.clone(), rate_limit::middleware))
        .layer(from_fn_with_state(platform, auth::middleware));
    if !cors_origins.is_empty() {
        router = router.layer(cors(&cors_origins));
    }
    router
        .layer(from_fn_with_state(security, security_headers::middleware))
        .layer(from_fn(request_id::middleware))
}
