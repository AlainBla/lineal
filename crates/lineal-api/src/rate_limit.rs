//! Token-Bucket-Rate-Limit (docs/09 §3): pro Identität zwei Buckets (Requests, Komplexität),
//! kontinuierliche Nachfüllung, `X-RateLimit-*`-Header und 429 mit `Retry-After`.
//! Design: Stufe 1 (`ip_middleware`, vor dem Token-Lookup) begrenzt je Client-IP, aber nur
//! Anfragen OHNE Zugangsdaten. Stufe 2 (`middleware`, nach dem Lookup) begrenzt gültig
//! authentifizierte Anfragen je User/Token (nie je IP: Büro-NAT) und belastet die IP nur mit
//! Anfragen, deren Zugangsdaten ungültig sind. Die Bucket-Map ist begrenzt; bei voller Map
//! teilen sich neue Schlüssel einen Überlauf-Bucket, aktive Buckets werden nie verdrängt.
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
use std::net::{IpAddr, SocketAddr};
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
    /// Anfragen je Client-IP (/64 bei IPv6) und Periode, deren Zugangsdaten sich als ungültig
    /// erweisen (erfundenes/abgelaufenes Token). Gültige Anfragen zählen NICHT gegen die IP,
    /// sondern gegen ihren User/Token (B3.01: Cookie-Sessions hinter NAT teilen sich sonst ein
    /// Limit). Zusätzlich sperrt `auth_failures` die IP bei vielen Fehlversuchen je Minute.
    pub ip_credentialed: f64,
    /// Grobes Sicherheitsnetz je Client-IP und Periode für ALLE Anfragen mit Zugangsdaten,
    /// gezählt vor dem Token-Lookup (egal ob gültig). Bewusst sehr hoch (Büro-NAT bleibt
    /// nutzbar), fängt aber Floods mit gültigen/ungültigen Credentials einer IP ab.
    pub ip_authenticated: f64,
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
            ip_credentialed: 2_000.0,
            ip_authenticated: 200_000.0,
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

/// Harte Obergrenze der Bucket-Anzahl (Speicherschutz gegen viele erfundene Identitäten).
pub const MAX_BUCKETS: usize = 200_000;

/// Gemeinsamer Bucket je Klasse für Schlüssel, die nicht mehr in die volle Map passen.
const OVERFLOW_PREFIX: &str = "\0overflow:";
/// Mindestabstand der Bereinigung voller Buckets (sonst O(n) pro Request unter Angriff).
const GC_INTERVAL_MS: i64 = 1_000;

#[derive(Default)]
struct State {
    buckets: HashMap<String, Bucket>,
    last_gc_ms: i64,
    last_overflow_warn_ms: i64,
}

/// Prozessweiter Bucket-Speicher.
///
/// Die Map ist hart begrenzt. Verdrängt wird **nie** ein Bucket mit Verbrauch (das würde dem
/// Besitzer volle Tokens zurückgeben, und ein Angreifer könnte seinen Bucket durch viele
/// Fake-Schlüssel "zurücksetzen"). Stattdessen werden nur Buckets entfernt, die ohnehin wieder
/// voll wären; ist danach immer noch kein Platz, teilen sich alle neuen Schlüssel einen
/// gemeinsamen Overflow-Bucket, bestehende Schlüssel behalten ihren Stand.
pub struct RateLimiter {
    state: Mutex<State>,
    max_buckets: usize,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::with_max_buckets(MAX_BUCKETS)
    }
}

/// Bucket-Schlüssel einer Client-IP: IPv4 pro Adresse (/32), IPv6 pro /64 (sonst umgeht ein
/// Client das Limit durch Adresswechsel innerhalb seines Präfixes); IPv4-mapped IPv6 wie IPv4.
pub fn ip_bucket(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => format!("v4:{v4}"),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => format!("v4:{v4}"),
            None => {
                let s = v6.segments();
                format!("v6:{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

impl RateLimiter {
    pub fn with_max_buckets(max_buckets: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            max_buckets: max_buckets.max(10),
        }
    }

    /// Anzahl gehaltener Buckets (Tests, Metriken).
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .buckets
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

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
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let state = &mut *state;
        let map = &mut state.buckets;
        if map.len() >= self.max_buckets && now_ms - state.last_gc_ms >= GC_INTERVAL_MS {
            state.last_gc_ms = now_ms;
            // Volle Buckets sind von neuen nicht zu unterscheiden: verwerfen.
            map.retain(|_, b| {
                b.tokens + (now_ms - b.updated_ms).max(0) as f64 / 1000.0 * rate < capacity
            });
        }
        // Map voll und Schlüssel unbekannt: kein eigener Bucket, sondern der gemeinsame
        // Überlauf-Bucket der Klasse (Präfix vor dem ersten ':') mit einem Zehntel des Limits.
        let overflow_key;
        let (key, capacity, rate) = if map.len() >= self.max_buckets && !map.contains_key(key) {
            if now_ms - state.last_overflow_warn_ms >= 60_000 {
                state.last_overflow_warn_ms = now_ms;
                tracing::warn!("rate limit table full; new keys share the overflow bucket");
            }
            overflow_key = format!("{OVERFLOW_PREFIX}{}", key.split(':').next().unwrap_or(""));
            let capacity = (capacity / 10.0).max(1.0);
            (overflow_key.as_str(), capacity, capacity / period_secs)
        } else {
            (key, capacity, rate)
        };
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

/// 429-Antwort passend zum Pfad (GraphQL-Fehlerformat bzw. REST-Fehlermodell).
fn rejection(req: &Request, d: &Decision, what: &str) -> Response {
    let err = denied_error(d, what);
    let mut res = if req.uri().path() == "/graphql" {
        graphql_error_response(&err)
    } else {
        ApiError(err).into_response()
    };
    requests_headers(res.headers_mut(), d);
    res
}

/// Bucket-Schlüssel der Client-IP (IPv6 pro /64); `None`, wenn die Peer-Adresse unbekannt ist
/// (kein gemeinsamer "unknown"-Bucket für alle).
pub(crate) fn client_ip(req: &Request) -> Option<String> {
    req.extensions()
        .get::<crate::client_ip::ClientIp>()
        .map(|c| c.0)
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip())
        })
        .map(ip_bucket)
}

fn exempt(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz")
}

/// Stufe 1 (vor der Authentifizierung): Anfragen ohne Zugangsdaten nutzen das (kleine)
/// Anonym-Limit je IP. Anfragen mit Zugangsdaten werden in Stufe 2 nach dem Lookup gezählt.
pub async fn ip_middleware(
    axum::extract::State(p): axum::extract::State<Platform>,
    req: Request,
    next: Next,
) -> Response {
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let Some(ip) = client_ip(&req) else {
        if p.cfg.require_peer {
            // Produktiv liefert `serve` immer die Peer-Adresse; fehlt sie, ist die Verdrahtung
            // kaputt. Ablehnen statt alle in einen gemeinsamen Bucket zu werfen.
            return ApiError(AppError::internal("Peer address unavailable.")).into_response();
        }
        // Nur Router-Tests (`oneshot`) haben keinen Peer: dort wird nicht nach IP begrenzt.
        return next.run(req).await;
    };
    // Anfragen mit Zugangsdaten: vor dem Lookup nur das hohe Sicherheitsnetz je IP; danach in
    // Stufe 2 gültig -> User-Bucket, ungültig -> strenger IP-Bucket. Brute-Force bremst
    // `auth_failures` schon vor dem Lookup.
    if crate::middleware::auth::extract_token(req.headers()).is_some() {
        let now_ms = p.app.clock.now().timestamp_millis();
        let d = p.limiter.take(
            &format!("ipa:{ip}"),
            p.cfg.rate.ip_authenticated,
            p.cfg.rate.period_secs,
            1.0,
            now_ms,
        );
        if !d.allowed {
            return rejection(&req, &d, "requests per IP");
        }
        return next.run(req).await;
    }
    let (key, capacity) = (format!("ip:{ip}"), p.cfg.rate.anonymous.requests);
    let now_ms = p.app.clock.now().timestamp_millis();
    let decision = p
        .limiter
        .take(&key, capacity, p.cfg.rate.period_secs, 1.0, now_ms);
    if !decision.allowed {
        return rejection(&req, &decision, "requests per IP");
    }
    let mut res = next.run(req).await;
    // Authentifizierte Anfragen tragen die Header ihres Token-Buckets (Stufe 2).
    if !res.headers().contains_key("x-ratelimit-requests-limit") {
        requests_headers(res.headers_mut(), &decision);
    }
    res
}

/// Stufe 2 (nach der Authentifizierung): Token-/User-Bucket (Requests) und `RateHandle` für die
/// GraphQL-Komplexität. Der `AuthState` ist nach dem Lookup unveränderlich.
pub async fn middleware(
    axum::extract::State(p): axum::extract::State<Platform>,
    mut req: Request,
    next: Next,
) -> Response {
    if exempt(req.uri().path()) {
        return next.run(req).await;
    }
    if matches!(
        req.extensions().get::<AuthState>(),
        Some(AuthState::Invalid)
    ) && let Some(ip) = client_ip(&req)
    {
        // Ungültige Zugangsdaten kosten IP-Kontingent (Token-Raten, abgelaufene Cookies).
        let now_ms = p.app.clock.now().timestamp_millis();
        let d = p.limiter.take(
            &format!("ipc:{ip}"),
            p.cfg.rate.ip_credentialed,
            p.cfg.rate.period_secs,
            1.0,
            now_ms,
        );
        if !d.allowed {
            return rejection(&req, &d, "requests per IP");
        }
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
        // Anonym/ungültig: bereits von Stufe 1 (IP) begrenzt.
        _ => return next.run(req).await,
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
    fn ipv6_clients_share_a_bucket_per_64_prefix() {
        let ip = |s: &str| ip_bucket(s.parse().unwrap());
        assert_eq!(
            ip("2001:db8:1:2::1"),
            ip("2001:db8:1:2:ffff:ffff:ffff:ffff")
        );
        assert_ne!(ip("2001:db8:1:2::1"), ip("2001:db8:1:3::1"));
        assert_ne!(ip("192.0.2.1"), ip("192.0.2.2"));
        assert_eq!(ip("::ffff:192.0.2.1"), ip("192.0.2.1"));
    }

    #[test]
    fn bucket_map_is_bounded_and_eviction_never_refills_a_drained_bucket() {
        let l = RateLimiter::with_max_buckets(100);
        let t = 1_000;
        // Der Angreifer verbraucht sein Kontingent (Kapazität 3 pro Stunde).
        for _ in 0..3 {
            assert!(l.take("ip:attacker", 3.0, 3600.0, 1.0, t).allowed);
        }
        assert!(!l.take("ip:attacker", 3.0, 3600.0, 1.0, t).allowed);
        // Er flutet die Map mit erfundenen Schlüsseln (z. B. gefälschte Client-IPs).
        for i in 0..1_000 {
            l.take(&format!("ip:fake{i}"), 3.0, 3600.0, 1.0, t + i64::from(i));
            assert!(l.len() <= 101, "len {} bei {i}", l.len());
        }
        // Sein eigener Bucket wurde nicht verdrängt: weiterhin gedrosselt.
        let d = l.take("ip:attacker", 3.0, 3600.0, 1.0, t + 2_000);
        assert!(
            !d.allowed,
            "Verdrängung darf keine vollen Tokens zurückgeben"
        );
        // Neue Schlüssel teilen sich bei voller Map den Overflow-Bucket und werden dort begrenzt.
        let late = (0..10)
            .filter(|i| {
                l.take(&format!("ip:late{i}"), 3.0, 3600.0, 1.0, t + 3_000)
                    .allowed
            })
            .count();
        assert_eq!(
            late, 0,
            "Überlauf-Bucket (1/10 des Limits, geteilt) ist erschöpft"
        );
        // Nach der natürlichen Auffüllzeit ist der Angreifer wieder frei.
        assert!(
            l.take("ip:attacker", 3.0, 3600.0, 1.0, t + 3_700_000)
                .allowed
        );
    }

    #[test]
    fn concurrent_requests_never_exceed_the_capacity() {
        let l = Arc::new(RateLimiter::default());
        let allowed: usize = (0..50)
            .map(|_| {
                let l = l.clone();
                std::thread::spawn(move || {
                    (0..20)
                        .filter(|_| l.take("ip:shared", 10.0, 3600.0, 1.0, 1_000).allowed)
                        .count()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .sum();
        assert_eq!(allowed, 10, "Prüfen und Abziehen sind atomar");
    }

    #[test]
    fn full_table_sends_new_account_keys_to_a_strict_shared_bucket() {
        let l = RateLimiter::with_max_buckets(10);
        for i in 0..10 {
            assert!(
                l.take(&format!("req:user:u{i}"), 100.0, 3600.0, 1.0, 0)
                    .allowed
            );
        }
        // Neue Konten (z. B. massenhaft erzeugte Sessions/Keys) teilen sich 1/10 des Limits.
        let ok = (0..50)
            .filter(|i| {
                l.take(&format!("req:user:new{i}"), 100.0, 3600.0, 1.0, 1)
                    .allowed
            })
            .count();
        assert_eq!(ok, 10);
        assert!(l.len() <= 11);
        // Bestehende Konten behalten ihren Stand.
        assert!(l.take("req:user:u0", 100.0, 3600.0, 1.0, 2).allowed);
    }

    #[test]
    fn reset_is_time_until_full() {
        let l = RateLimiter::default();
        let d = l.take("k", 10.0, 10.0, 4.0, 1_000);
        assert_eq!(d.remaining, 6);
        assert_eq!(d.reset_ms, 1_000 + 4_000);
    }
}
