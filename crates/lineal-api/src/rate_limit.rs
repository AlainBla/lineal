//! Token-Bucket-Rate-Limit (docs/09 §3): pro Identität zwei Buckets (Requests, Komplexität),
//! kontinuierliche Nachfüllung, `X-RateLimit-*`-Header und 429 mit `Retry-After`.
//! Zustand liegt im Prozessspeicher (eine Instanz); die Zeit kommt aus der `Clock` der `App`.

use crate::error::{ApiError, graphql_error_response};
use crate::middleware::Platform;
use crate::middleware::auth::{AuthState, CredentialClass};
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lineal_app::AppError;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Limits einer Identitätsklasse pro Periode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassLimits {
    pub requests: f64,
    pub complexity: f64,
}

/// Konfiguration; Defaults aus 09 §3.1.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimits {
    pub period_secs: f64,
    pub api_key: ClassLimits,
    pub oauth: ClassLimits,
    /// Web-App über Session-Cookie: 10x OAuth.
    pub session: ClassLimits,
    pub anonymous: ClassLimits,
    /// Einzelquery über diesem Wert wird abgelehnt.
    pub max_query_complexity: u64,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            period_secs: 3600.0,
            api_key: ClassLimits {
                requests: 2_500.0,
                complexity: 3_000_000.0,
            },
            oauth: ClassLimits {
                requests: 5_000.0,
                complexity: 2_000_000.0,
            },
            session: ClassLimits {
                requests: 50_000.0,
                complexity: 20_000_000.0,
            },
            anonymous: ClassLimits {
                requests: 600.0,
                complexity: 100_000.0,
            },
            max_query_complexity: 10_000,
        }
    }
}

impl RateLimits {
    pub fn for_class(&self, class: CredentialClass) -> ClassLimits {
        match class {
            CredentialClass::Anonymous => self.anonymous,
            CredentialClass::ApiKey => self.api_key,
            CredentialClass::OAuth => self.oauth,
            CredentialClass::Session => self.session,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated_ms: i64,
}

/// Ergebnis eines `take`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    /// Unix-Zeit (ms), zu der der Bucket wieder voll ist.
    pub reset_ms: i64,
    /// Sekunden bis genug Tokens da sind (nur bei `allowed = false` relevant).
    pub retry_after_secs: u64,
}

const GC_THRESHOLD: usize = 10_000;

/// Prozessweiter Bucket-Speicher.
#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// Entnimmt `cost` Tokens aus dem Bucket `key` (atomar unter dem Mutex).
    pub fn take(
        &self,
        key: &str,
        capacity: f64,
        period_secs: f64,
        cost: f64,
        now_ms: i64,
    ) -> Decision {
        let rate = capacity / period_secs;
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if map.len() > GC_THRESHOLD {
            // Volle Buckets sind von neuen nicht zu unterscheiden: verwerfen.
            map.retain(|_, b| {
                b.tokens + (now_ms - b.updated_ms).max(0) as f64 / 1000.0 * rate < capacity
            });
        }
        let b = map.entry(key.to_owned()).or_insert(Bucket {
            tokens: capacity,
            updated_ms: now_ms,
        });
        let elapsed = (now_ms - b.updated_ms).max(0) as f64 / 1000.0;
        b.tokens = (b.tokens + elapsed * rate).min(capacity);
        b.updated_ms = now_ms;
        let allowed = b.tokens >= cost;
        let retry_after_secs = if allowed {
            0
        } else {
            ((cost - b.tokens) / rate).ceil().max(1.0) as u64
        };
        if allowed {
            b.tokens -= cost;
        }
        let reset_ms = now_ms + ((capacity - b.tokens) / rate * 1000.0).ceil() as i64;
        Decision {
            allowed,
            limit: capacity as u64,
            remaining: b.tokens.floor().max(0.0) as u64,
            reset_ms,
            retry_after_secs,
        }
    }
}

/// Pro Request von der Middleware an die Handler gereicht (GraphQL-Komplexität).
#[derive(Clone)]
pub struct RateHandle {
    pub limiter: Arc<RateLimiter>,
    pub key: String,
    pub limits: ClassLimits,
    pub period_secs: f64,
    pub max_query_complexity: u64,
    pub now_ms: i64,
}

impl RateHandle {
    pub fn take_complexity(&self, cost: u64) -> Decision {
        self.limiter.take(
            &format!("cplx:{}", self.key),
            self.limits.complexity,
            self.period_secs,
            cost as f64,
            self.now_ms,
        )
    }
}

fn set(headers: &mut HeaderMap, name: &'static str, value: impl ToString) {
    if let Ok(v) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(HeaderName::from_static(name), v);
    }
}

/// `X-RateLimit-Requests-*`.
pub fn requests_headers(headers: &mut HeaderMap, d: &Decision) {
    set(headers, "x-ratelimit-requests-limit", d.limit);
    set(headers, "x-ratelimit-requests-remaining", d.remaining);
    set(headers, "x-ratelimit-requests-reset", d.reset_ms);
}

/// `X-RateLimit-Complexity-*` und `X-Complexity`.
pub fn complexity_headers(headers: &mut HeaderMap, d: &Decision, complexity: u64) {
    set(headers, "x-complexity", complexity);
    set(headers, "x-ratelimit-complexity-limit", d.limit);
    set(headers, "x-ratelimit-complexity-remaining", d.remaining);
    set(headers, "x-ratelimit-complexity-reset", d.reset_ms);
}

/// 429 `rate_limited` mit `Retry-After`.
pub fn denied_error(d: &Decision, what: &str) -> AppError {
    AppError::rate_limited(format!("Rate limit exceeded ({what})."))
        .with_details(json!({ "retryAfter": d.retry_after_secs, "limit": d.limit }))
}

fn client_ip(req: &Request) -> String {
    req.extensions()
        .get::<crate::client_ip::ClientIp>()
        .map(|c| c.0)
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip())
        })
        .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string())
}

fn exempt(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz")
}

/// Middleware: belastet den Request-Bucket, setzt Header, antwortet bei Überschreitung mit 429.
pub async fn middleware(
    axum::extract::State(p): axum::extract::State<Platform>,
    mut req: Request,
    next: Next,
) -> Response {
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let (class, key) = match req.extensions().get::<AuthState>() {
        Some(AuthState::Authenticated { ctx, class }) => {
            let id = match ctx.actor {
                lineal_app::Actor::User { id, .. } | lineal_app::Actor::App { id } => {
                    id.to_string()
                }
                lineal_app::Actor::System => ctx.org_id.to_string(),
            };
            (*class, format!("user:{id}"))
        }
        _ => (
            CredentialClass::Anonymous,
            format!("ip:{}", client_ip(&req)),
        ),
    };
    let limits = p.cfg.rate.for_class(class);
    let now_ms = p.app.clock.now().timestamp_millis();
    let decision = p.limiter.take(
        &format!("req:{key}"),
        limits.requests,
        p.cfg.rate.period_secs,
        1.0,
        now_ms,
    );
    let is_graphql = req.uri().path() == "/graphql";
    if !decision.allowed {
        let err = denied_error(&decision, "requests");
        let mut res = if is_graphql {
            graphql_error_response(&err)
        } else {
            ApiError(err).into_response()
        };
        requests_headers(res.headers_mut(), &decision);
        return res;
    }
    req.extensions_mut().insert(RateHandle {
        limiter: p.limiter.clone(),
        key,
        limits,
        period_secs: p.cfg.rate.period_secs,
        max_query_complexity: p.cfg.rate.max_query_complexity,
        now_ms,
    });
    let mut res = next.run(req).await;
    requests_headers(res.headers_mut(), &decision);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_drains_refills_and_reports_retry_after() {
        let l = RateLimiter::default();
        // Kapazität 2 pro 10 s => 0,2 Token/s
        let d1 = l.take("k", 2.0, 10.0, 1.0, 0);
        assert!(d1.allowed);
        assert_eq!(d1.remaining, 1);
        assert!(l.take("k", 2.0, 10.0, 1.0, 0).allowed);
        let d3 = l.take("k", 2.0, 10.0, 1.0, 0);
        assert!(!d3.allowed);
        assert_eq!(d3.retry_after_secs, 5);
        // nach 5 s ist wieder ein Token da
        assert!(l.take("k", 2.0, 10.0, 1.0, 5_000).allowed);
        // andere Schlüssel sind unabhängig
        assert!(l.take("other", 2.0, 10.0, 1.0, 0).allowed);
    }

    #[test]
    fn reset_is_time_until_full() {
        let l = RateLimiter::default();
        let d = l.take("k", 10.0, 10.0, 4.0, 1_000);
        assert_eq!(d.remaining, 6);
        assert_eq!(d.reset_ms, 1_000 + 4_000);
    }
}
