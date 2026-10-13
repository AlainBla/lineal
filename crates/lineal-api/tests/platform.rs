//! Integrationstests der API-Plattform (WP03): Fehlermodell, Auth, Rate-Limit, Security-Header,
//! CORS, signierte Blob-URLs. Alle Daten erfunden, keine Datenbank (FakeStore).

#![allow(clippy::unwrap_used)]

use async_graphql::{Context, EmptySubscription, Object, Schema};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use lineal_api::blob::{GET_TTL, LocalBlobStore, PUT_TTL};
use lineal_api::error::{ApiError, GqlResultExt, gql_error};
use lineal_api::middleware::auth::{Authenticated, hash_token};
use lineal_api::middleware::{PlatformConfig, apply_with};
use lineal_api::rate_limit::{ClassLimits, RateLimits};
use lineal_app::ports::repos::Principal;
use lineal_app::ports::{BlobStore, Clock};
use lineal_app::{Actor, App, AppError, ErrorKind, Scopes};
use lineal_domain::ids::{OrgId, UserId};
use lineal_testkit::{FakeStore, test_app};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const KEY_RW: &str = "lin_api_alice_readwrite";
const KEY_RO: &str = "lin_api_bob_readonly";

fn principal(scopes: Scopes) -> Principal {
    Principal {
        kind: lineal_app::ports::repos::CredentialKind::ApiKey,
        org_id: OrgId::new_v7(),
        actor: Actor::App {
            id: UserId::new_v7(),
        },
        scopes,
        team_scope: None,
    }
}

fn app_with_keys() -> Arc<App> {
    let store = FakeStore::new()
        .with_principal(hash_token(KEY_RW), principal(Scopes::read_write()))
        .with_principal(hash_token(KEY_RO), principal(Scopes::read_only()))
        .with_principal(hash_token("lin_oauth_token"), {
            let mut p = principal(Scopes::all());
            p.kind = lineal_app::ports::repos::CredentialKind::OAuth;
            p
        })
        .with_principal(hash_token("session-token"), {
            let mut p = principal(Scopes::all());
            p.kind = lineal_app::ports::repos::CredentialKind::Session;
            p
        });
    test_app(Arc::new(store))
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn send(router: &Router, req: Request<Body>) -> Reply {
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn gql(auth: Option<(&str, &str)>, query: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/graphql")
        .header("content-type", "application/json");
    if let Some((name, value)) = auth {
        b = b.header(name, value);
    }
    b.body(Body::from(json!({ "query": query }).to_string()))
        .unwrap()
}

/// Browser-typische Header einer Same-Origin-Anfrage der Web-App (CSRF-Schutz, Basis-URL
/// `http://localhost:8080` ist der Default der Test-App).
fn same_origin(mut req: Request<Body>) -> Request<Body> {
    let h = req.headers_mut();
    h.insert("origin", "http://localhost:8080".parse().unwrap());
    h.insert("x-lineal-csrf", "1".parse().unwrap());
    req
}

fn api(app: Arc<App>) -> Router {
    lineal_api::router(app, false)
}

/// Token-Art und Transport müssen zusammenpassen: Cookie nur für Sessions, `Authorization`
/// nur für API-Keys/OAuth-Tokens (sonst ließe sich die „nur Session“-Prüfung umgehen).
#[tokio::test]
async fn token_kind_must_match_transport() {
    let router = api(app_with_keys());
    for (name, value) in [
        ("cookie", format!("lineal_session={KEY_RW}")),
        ("cookie", "lineal_session=lin_oauth_token".to_owned()),
        ("authorization", "session-token".to_owned()),
        ("authorization", "Bearer session-token".to_owned()),
    ] {
        let r = send(
            &router,
            same_origin(gql(Some((name, &value)), "{ __typename }")),
        )
        .await;
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "{name}: {value}: {}",
            r.body
        );
        assert!(r.body.contains("unauthenticated"), "{}", r.body);
    }
    // Zulässige Kombinationen bleiben.
    for (name, value) in [
        ("cookie", "lineal_session=session-token".to_owned()),
        ("authorization", KEY_RW.to_owned()),
        ("authorization", "Bearer lin_oauth_token".to_owned()),
    ] {
        let r = send(
            &router,
            same_origin(gql(Some((name, &value)), "{ __typename }")),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{name}: {}", r.body);
    }
    // REST `/auth/me` prüft dieselbe Regel.
    for (name, value) in [
        ("cookie", format!("lineal_session={KEY_RW}")),
        ("cookie", "lineal_session=lin_oauth_token".to_owned()),
        ("authorization", "Bearer session-token".to_owned()),
    ] {
        let req = Request::builder()
            .method(Method::GET)
            .uri("/auth/me")
            .header(name, value.as_str())
            .body(Body::empty())
            .unwrap();
        let r = send(&router, req).await;
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "{name}: {value}: {}",
            r.body
        );
    }
}

// ---------------------------------------------------------------- Fehlermodell

#[tokio::test]
async fn error_model_snapshot_per_error_kind() {
    let expected = [
        (ErrorKind::InvalidRequest, 400, "invalid_request"),
        (ErrorKind::Unauthenticated, 401, "unauthenticated"),
        (ErrorKind::Forbidden, 403, "forbidden"),
        (ErrorKind::NotFound, 404, "not_found"),
        (ErrorKind::AmbiguousReference, 409, "ambiguous_reference"),
        (ErrorKind::Conflict, 409, "conflict"),
        (ErrorKind::LimitExceeded, 422, "limit_exceeded"),
        (ErrorKind::RateLimited, 429, "rate_limited"),
        (ErrorKind::Internal, 500, "internal"),
    ];
    assert_eq!(expected.len(), ErrorKind::ALL.len());
    for (kind, status, code) in expected {
        let mut err = AppError::new(kind, "Could not find referenced Issue.")
            .with_details(json!({ "reference": "ENG-999999" }));
        if kind == ErrorKind::RateLimited {
            err = err.with_details(json!({ "retryAfter": 12 }));
        }
        let res = ApiError(err.clone()).into_response();
        assert_eq!(res.status().as_u16(), status, "{code}");
        assert_eq!(
            res.headers().get("retry-after").is_some(),
            kind == ErrorKind::RateLimited
        );
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let message = if kind == ErrorKind::Internal {
            "Internal server error."
        } else {
            "Could not find referenced Issue."
        };
        assert_eq!(body["error"], code);
        assert_eq!(body["status"], status);
        assert_eq!(body["message"], message);
        assert_eq!(body["requestId"], "req_unknown");
        assert_eq!(body["details"], err.details);
        // GraphQL-Form: dieselben Felder unter extensions
        let g = gql_error(&err, "req_x");
        let ext = g.extensions.unwrap();
        assert_eq!(ext.get("code").unwrap().to_string(), format!("\"{code}\""));
        assert_eq!(ext.get("status").unwrap().to_string(), status.to_string());
        assert_eq!(ext.get("requestId").unwrap().to_string(), "\"req_x\"");
    }
}

struct Boom;

#[Object]
impl Boom {
    async fn missing(&self, ctx: &Context<'_>) -> async_graphql::Result<i32> {
        Err::<i32, _>(AppError::not_found("Could not find referenced Issue.")).gql(ctx)
    }
}

#[tokio::test]
async fn resolver_errors_carry_extensions_code() {
    let schema = Schema::build(Boom, async_graphql::EmptyMutation, EmptySubscription).finish();
    let ctx = lineal_testkit::ctx_for(lineal_testkit::SeedUser::Alice);
    let res = schema
        .execute(async_graphql::Request::new("{ missing }").data(ctx))
        .await;
    let v = serde_json::to_value(&res).unwrap();
    assert_eq!(v["errors"][0]["extensions"]["code"], "not_found");
    assert_eq!(v["errors"][0]["extensions"]["status"], 404);
    assert_eq!(v["errors"][0]["extensions"]["requestId"], "req_test_alice");
    assert_eq!(v["data"], Value::Null);
}

// ---------------------------------------------------------------- Auth

#[tokio::test]
async fn unauthenticated_graphql_is_401_with_code() {
    let router = api(app_with_keys());
    for auth in [
        None,
        Some(("authorization", "lin_api_unknown")),
        Some(("cookie", "lineal_session=bogus")),
    ] {
        let r = send(&router, same_origin(gql(auth, "{ __typename }"))).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{auth:?}");
        let v = r.json();
        assert_eq!(v["errors"][0]["extensions"]["code"], "unauthenticated");
        let rid = v["errors"][0]["extensions"]["requestId"].as_str().unwrap();
        assert!(rid.starts_with("req_"));
        assert_eq!(
            r.headers.get("x-request-id").unwrap().to_str().unwrap(),
            rid
        );
    }
}

#[tokio::test]
async fn csrf_protects_cookie_sessions_but_not_api_keys() {
    let router = api(app_with_keys());
    let cookie = Some(("cookie", "lineal_session=session-token"));
    let q = "{ __typename }";
    // gleiche Origin + Header: ok
    let r = send(&router, same_origin(gql(cookie, q))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // fremde Origin: 403
    let mut cross = same_origin(gql(cookie, q));
    cross
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let r = send(&router, cross).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.body);
    assert_eq!(r.json()["error"], "forbidden");
    // Origin ok, aber ohne CSRF-Header: 403
    let mut no_header = same_origin(gql(cookie, q));
    no_header.headers_mut().remove("x-lineal-csrf");
    assert_eq!(send(&router, no_header).await.status, StatusCode::FORBIDDEN);
    // Header, aber ohne Origin/Referer: 403
    let mut no_origin = same_origin(gql(cookie, q));
    no_origin.headers_mut().remove("origin");
    assert_eq!(send(&router, no_origin).await.status, StatusCode::FORBIDDEN);
    // Referer ersetzt Origin
    let mut referer = same_origin(gql(cookie, q));
    referer.headers_mut().remove("origin");
    referer.headers_mut().insert(
        "referer",
        "http://localhost:8080/acme/inbox".parse().unwrap(),
    );
    assert_eq!(send(&router, referer).await.status, StatusCode::OK);
    // API-Key ohne Header und ohne Origin: ok
    let r = send(&router, gql(Some(("authorization", KEY_RW)), q)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // Logout per Cookie braucht ebenfalls Origin + Header
    let logout = |same: bool| {
        let mut r = Request::builder()
            .method(Method::POST)
            .uri("/auth/logout")
            .header("cookie", "lineal_session=session-token")
            .body(Body::empty())
            .unwrap();
        if same {
            r = same_origin(r);
        }
        r
    };
    assert_eq!(
        send(&router, logout(false)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_ne!(
        send(&router, logout(true)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn token_sources_all_authenticate() {
    let router = api(app_with_keys());
    for auth in [
        ("authorization", KEY_RW.to_owned()),
        ("authorization", format!("Bearer {KEY_RW}")),
        (
            "cookie",
            "theme=dark; lineal_session=session-token".to_owned(),
        ),
    ] {
        let req = same_origin(gql(Some((auth.0, &auth.1)), "{ __typename }"));
        let r = send(&router, req).await;
        assert_eq!(r.status, StatusCode::OK, "{auth:?}: {}", r.body);
        assert_eq!(r.json()["data"]["__typename"], "Query");
        assert!(r.headers.contains_key("x-ratelimit-requests-limit"));
        assert!(r.headers.contains_key("x-complexity"));
    }
}

#[tokio::test]
async fn read_only_scope_cannot_mutate_and_errors_follow_the_model() {
    let router = api(app_with_keys());
    let m = "mutation { __typename }";
    let r = send(&router, gql(Some(("authorization", KEY_RO)), m)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["errors"][0]["extensions"]["code"], "forbidden");
    let r = send(&router, gql(Some(("authorization", KEY_RW)), m)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    // Syntaxfehler -> 400 invalid_request
    let r = send(
        &router,
        gql(Some(("authorization", KEY_RW)), "{ __typename "),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        r.json()["errors"][0]["extensions"]["code"],
        "invalid_request"
    );
    // unbekanntes Feld (Validierung) -> 400 invalid_request
    let r = send(
        &router,
        gql(Some(("authorization", KEY_RW)), "{ doesNotExist }"),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        r.json()["errors"][0]["extensions"]["code"],
        "invalid_request"
    );
}

#[tokio::test]
async fn too_complex_query_is_rejected_with_details() {
    let router = api(app_with_keys());
    let q = "{ issues(first: 250) { nodes { comments(first: 250) { nodes { children(first: 250) { nodes { id } } } } } } }";
    let r = send(&router, gql(Some(("authorization", KEY_RW)), q)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let ext = &r.json()["errors"][0]["extensions"];
    assert_eq!(ext["code"], "invalid_request");
    assert!(ext["details"]["complexity"].as_u64().unwrap() > 10_000);
}

#[tokio::test]
async fn authenticated_rest_extractor_answers_401_without_token() {
    async fn me(auth: Authenticated) -> String {
        auth.0.request_id
    }
    let router = apply_with(
        Router::new().route("/me", get(me)),
        app_with_keys(),
        PlatformConfig::new(&app_with_keys(), false),
    );
    let anon = Request::get("/me").body(Body::empty()).unwrap();
    let r = send(&router, anon).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["error"], "unauthenticated");
    let authed = Request::get("/me")
        .header("authorization", KEY_RW)
        .body(Body::empty())
        .unwrap();
    let r = send(&router, authed).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.starts_with("req_"));
}

// ---------------------------------------------------------------- Rate-Limit

fn limited_router(app: Arc<App>, requests: f64, complexity: f64) -> Router {
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate = RateLimits {
        anonymous: ClassLimits {
            requests,
            complexity,
        },
        api_key: ClassLimits {
            requests,
            complexity,
        },
        ..RateLimits::default()
    };
    apply_with(
        Router::new()
            .route("/ping", get(|| async { "pong" }))
            .route("/healthz", get(|| async { "ok" }))
            .route("/graphql", axum::routing::post(|| async { StatusCode::OK })),
        app,
        cfg,
    )
}

fn from_peer(peer: &str, forwarded: &str) -> Request<Body> {
    let mut req = Request::get("/ping")
        .header("x-forwarded-for", forwarded)
        .body(Body::empty())
        .unwrap();
    let addr: std::net::SocketAddr = format!("{peer}:4000").parse().unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(addr));
    req
}

#[tokio::test]
async fn forged_forwarded_for_from_untrusted_peer_does_not_evade_the_rate_limit() {
    let router = limited_router(app_with_keys(), 2.0, 100.0);
    // Jeder Request behauptet eine andere Client-IP, kommt aber vom selben Peer.
    for (i, expected) in [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::TOO_MANY_REQUESTS,
    ]
    .into_iter()
    .enumerate()
    {
        let r = send(&router, from_peer("203.0.113.9", &format!("1.2.3.{i}"))).await;
        assert_eq!(r.status, expected, "request {i}");
    }
}

#[tokio::test]
async fn forwarded_for_is_honoured_only_behind_a_trusted_proxy() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.anonymous = ClassLimits {
        requests: 1.0,
        complexity: 100.0,
    };
    cfg.trusted_proxies = lineal_api::client_ip::parse_list(&["10.0.0.0/8".to_owned()]);
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    // Zwei verschiedene Clients hinter dem Proxy haben getrennte Buckets ...
    assert_eq!(
        send(&router, from_peer("10.0.0.5", "198.51.100.1"))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, from_peer("10.0.0.5", "198.51.100.2"))
            .await
            .status,
        StatusCode::OK
    );
    // ... derselbe Client wird begrenzt, auch wenn er vorn einen gefälschten Eintrag anhängt.
    assert_eq!(
        send(&router, from_peer("10.0.0.5", "6.6.6.6, 198.51.100.1"))
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn invalid_forwarded_for_behind_a_trusted_proxy_is_rejected_not_pooled() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.trusted_proxies = lineal_api::client_ip::parse_list(&["10.0.0.0/8".to_owned()]);
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    // Der kaputte Header kostet nur diesen Request (400); er belastet keinen Bucket anderer.
    for _ in 0..3 {
        let r = send(&router, from_peer("10.0.0.5", "6.6.6.6, unknown")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
    }
    let ok = send(&router, from_peer("10.0.0.5", "198.51.100.1")).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.headers["x-ratelimit-requests-remaining"], "599");
}

#[tokio::test]
async fn requests_without_peer_address_never_share_a_global_bucket() {
    let ping = || Request::get("/ping").body(Body::empty()).unwrap();
    let make = |require_peer: bool| {
        let app = app_with_keys();
        let mut cfg = PlatformConfig::new(&app, false);
        cfg.require_peer = require_peer;
        cfg.rate.anonymous = ClassLimits {
            requests: 1.0,
            complexity: 100.0,
        };
        apply_with(
            Router::new().route("/ping", get(|| async { "pong" })),
            app,
            cfg,
        )
    };
    // Produktiv (require_peer): ohne Peer abgelehnt statt in einem "unknown"-Bucket gedrosselt.
    let strict = make(true);
    for _ in 0..2 {
        assert_eq!(
            send(&strict, ping()).await.status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    // Tests: ohne Peer wird nicht anonym begrenzt (kein gemeinsamer Bucket).
    let lax = make(false);
    for _ in 0..3 {
        assert_eq!(send(&lax, ping()).await.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn ipv6_privacy_addresses_of_one_prefix_share_the_rate_limit() {
    let router = limited_router(app_with_keys(), 2.0, 100.0);
    let from6 = |ip: &str| {
        let mut req = Request::get("/ping").body(Body::empty()).unwrap();
        let addr: std::net::SocketAddr = format!("[{ip}]:4000").parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        req
    };
    assert_eq!(
        send(&router, from6("2001:db8:1:2::1")).await.status,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, from6("2001:db8:1:2::2")).await.status,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, from6("2001:db8:1:2:aaaa::3")).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Anderes /64: eigener Bucket.
    assert_eq!(
        send(&router, from6("2001:db8:1:9::1")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn switching_tokens_does_not_evade_the_ip_limit() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.ip_credentialed = 3.0;
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    let guess = |peer: &str, n: usize| {
        let mut req = from_peer(peer, "");
        req.headers_mut().insert(
            "authorization",
            format!("lin_api_guess{n}").parse().unwrap(),
        );
        req
    };
    // Jeder Versuch mit einem anderen erfundenen Token kostet IP-Kontingent (ungültige
    // Zugangsdaten); nach drei Versuchen ist Schluss, mit JSON-Fehler und Retry-After.
    for n in 0..3 {
        assert_eq!(
            send(&router, guess("203.0.113.5", n)).await.status,
            StatusCode::OK
        );
    }
    let r = send(&router, guess("203.0.113.5", 3)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.body);
    assert_eq!(r.json()["error"], "rate_limited");
    assert!(r.headers.contains_key("retry-after"));
    // B3.01: Ein GÜLTIGES Token von derselben IP (Büro-NAT) wird davon nicht getroffen, es
    // zählt gegen seinen User-Bucket; eine andere IP ist ebenfalls frei.
    let mut valid = from_peer("203.0.113.5", "");
    valid
        .headers_mut()
        .insert("authorization", KEY_RW.parse().unwrap());
    assert_eq!(send(&router, valid).await.status, StatusCode::OK);
    assert_eq!(
        send(&router, guess("203.0.113.6", 0)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn valid_credentials_are_limited_per_account_not_per_ip() {
    // Mehr gültige Anfragen als jedes IP-Limit zulässt: alle kommen durch; erst das Konto-
    // Limit greift, und nur für dieses Konto (zweiter Key von derselben IP bleibt frei).
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.anonymous.requests = 2.0;
    cfg.rate.ip_credentialed = 2.0;
    cfg.rate.api_key = ClassLimits {
        requests: 6.0,
        complexity: 100.0,
    };
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    let with = |key: &str| {
        let mut req = from_peer("203.0.113.7", "");
        req.headers_mut()
            .insert("authorization", key.parse().unwrap());
        req
    };
    for i in 0..6 {
        assert_eq!(
            send(&router, with(KEY_RW)).await.status,
            StatusCode::OK,
            "{i}"
        );
    }
    let r = send(&router, with(KEY_RW)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.json()["error"], "rate_limited");
    assert!(r.headers.contains_key("retry-after"));
    assert_eq!(send(&router, with(KEY_RO)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn coarse_ip_safety_net_applies_to_authenticated_requests_before_the_lookup() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.ip_authenticated = 3.0;
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    let with = |peer: &str, key: &str| {
        let mut req = from_peer(peer, "");
        req.headers_mut()
            .insert("authorization", key.parse().unwrap());
        req
    };
    for _ in 0..3 {
        assert_eq!(
            send(&router, with("203.0.113.8", KEY_RW)).await.status,
            StatusCode::OK
        );
    }
    // Weder gültige noch erfundene Tokens umgehen das Netz; JSON mit Retry-After.
    let r = send(&router, with("203.0.113.8", KEY_RO)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers.contains_key("retry-after"));
    assert_eq!(
        send(&router, with("203.0.113.8", "lin_api_fake"))
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        send(&router, with("203.0.113.9", KEY_RW)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn credentialed_requests_without_peer_fail_closed_in_production() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.require_peer = true;
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    for auth in [KEY_RW, "lin_api_fake"] {
        let req = Request::get("/ping")
            .header("authorization", auth)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            send(&router, req).await.status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}

#[tokio::test]
async fn parallel_requests_never_exceed_the_account_budget() {
    // (a) Prüfen und Abziehen sind atomar: 40 gleichzeitige Anfragen, Budget 10.
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.api_key = ClassLimits {
        requests: 10.0,
        complexity: 100.0,
    };
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    let mut tasks = Vec::new();
    for _ in 0..40 {
        let router = router.clone();
        tasks.push(tokio::spawn(async move {
            let mut req = from_peer("203.0.113.30", "");
            req.headers_mut()
                .insert("authorization", KEY_RW.parse().unwrap());
            send(&router, req).await.status
        }));
    }
    let mut ok = 0;
    for t in tasks {
        if t.await.unwrap() == StatusCode::OK {
            ok += 1;
        }
    }
    assert_eq!(ok, 10);
}

#[tokio::test]
async fn every_adapter_path_is_limited_and_scheme_or_case_changes_do_not_evade() {
    // (b) /mcp, /sync, /collab, /oauth/token, /invite/accept, /files, /uploads laufen durch
    // dieselbe Plattform-Middleware wie /graphql; (d) Schema-/Schreibweisenwechsel führen zum
    // selben Konto-Bucket bzw. zählen als ungültig gegen die IP.
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.anonymous.requests = 1.0;
    cfg.rate.ip_credentialed = 2.0;
    cfg.rate.api_key = ClassLimits {
        requests: 5.0,
        complexity: 100.0,
    };
    let paths = [
        "/mcp",
        "/sync",
        "/sync/bootstrap",
        "/collab/doc",
        "/oauth/token",
        "/invite/accept",
        "/files/x",
        "/uploads/x",
    ];
    let mut r = Router::new();
    for p in paths {
        r = r.route(p, axum::routing::any(|| async { "ok" }));
    }
    let router = apply_with(r, app, cfg);
    let req = |peer: &str, path: &str, auth: Option<&str>| {
        let mut req = from_peer(peer, "");
        *req.uri_mut() = path.parse().unwrap();
        if let Some(a) = auth {
            req.headers_mut()
                .insert("authorization", a.parse().unwrap());
        }
        req
    };
    // Anonym: jeder Pfad teilt das IP-Limit (1).
    assert_eq!(
        send(&router, req("203.0.113.40", "/mcp", None))
            .await
            .status,
        StatusCode::OK
    );
    for p in paths {
        assert_eq!(
            send(&router, req("203.0.113.40", p, None)).await.status,
            StatusCode::TOO_MANY_REQUESTS,
            "{p}"
        );
    }
    // Gültiger Key in vier Schreibweisen: ein Konto-Bucket über alle Pfade (5).
    let variants = [
        format!("Bearer {KEY_RW}"),
        format!("bearer {KEY_RW}"),
        format!("BEARER  {KEY_RW}"),
        KEY_RW.to_string(),
        format!("Bearer {KEY_RW}"),
    ];
    for (i, v) in variants.iter().enumerate() {
        let r = send(&router, req("203.0.113.41", paths[i], Some(v))).await;
        assert_eq!(r.status, StatusCode::OK, "{v}");
    }
    let r = send(&router, req("203.0.113.41", "/mcp", Some(KEY_RW))).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // Fremdes Schema/erfundene Werte: gelten als ungültige Zugangsdaten, zählen gegen die IP
    // (hier 2) und kosten keinen Konto-Bucket.
    let bogus = ["Basic Zm9vOmJhcg==", "Token abc", "Digest x"];
    let mut codes = Vec::new();
    for b in bogus {
        codes.push(
            send(&router, req("203.0.113.42", "/mcp", Some(b)))
                .await
                .status,
        );
    }
    assert_eq!(codes[2], StatusCode::TOO_MANY_REQUESTS, "{codes:?}");
}

#[tokio::test]
async fn forged_forwarded_for_does_not_evade_credentialed_ip_limits() {
    // (c) Ein untrusted Peer kann per X-Forwarded-For weder das Netz (ip_authenticated) noch
    // das Ungültig-Limit auf andere "IPs" verteilen.
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.ip_authenticated = 3.0;
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    let mut statuses = Vec::new();
    for i in 0..5 {
        let mut req = from_peer("203.0.113.50", &format!("9.9.9.{i}"));
        req.headers_mut()
            .insert("authorization", format!("lin_api_fake{i}").parse().unwrap());
        statuses.push(send(&router, req).await.status);
    }
    assert_eq!(statuses[3], StatusCode::TOO_MANY_REQUESTS, "{statuses:?}");
}

#[tokio::test]
async fn many_keys_of_one_account_share_one_bucket() {
    // Gleicher Principal über zwei verschiedene Tokens: ein gemeinsamer Konto-Bucket.
    let shared = principal(Scopes::read_write());
    let store = FakeStore::new()
        .with_principal(hash_token("lin_api_k1"), shared.clone())
        .with_principal(hash_token("lin_api_k2"), shared);
    let app = test_app(Arc::new(store));
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.api_key = ClassLimits {
        requests: 4.0,
        complexity: 100.0,
    };
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    for i in 0..4 {
        let key = if i % 2 == 0 {
            "lin_api_k1"
        } else {
            "lin_api_k2"
        };
        let mut req = from_peer("203.0.113.20", "");
        req.headers_mut()
            .insert("authorization", key.parse().unwrap());
        assert_eq!(send(&router, req).await.status, StatusCode::OK);
    }
    let mut req = from_peer("203.0.113.20", "");
    req.headers_mut()
        .insert("authorization", "lin_api_k1".parse().unwrap());
    assert_eq!(
        send(&router, req).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn cookie_sessions_behind_one_ip_are_not_throttled_by_the_ip_limit() {
    // Web-App hinter NAT: viele Seitenaufrufe mit gültigem Cookie von einer IP. Das IP-Limit
    // (hier 2) greift nicht; das Session-Limit gilt je Konto.
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.anonymous.requests = 2.0;
    cfg.rate.ip_credentialed = 2.0;
    let router = apply_with(
        Router::new().route("/ping", get(|| async { "pong" })),
        app,
        cfg,
    );
    for i in 0..20 {
        let mut req = from_peer("198.51.100.77", "");
        req.headers_mut()
            .insert("cookie", "lineal_session=session-token".parse().unwrap());
        assert_eq!(send(&router, req).await.status, StatusCode::OK, "{i}");
    }
}

#[tokio::test]
async fn request_rate_limit_returns_429_with_headers() {
    let router = limited_router(app_with_keys(), 3.0, 100.0);
    let ping = || from_peer("203.0.113.5", "");
    for remaining in [2, 1, 0] {
        let r = send(&router, ping()).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.headers["x-ratelimit-requests-limit"], "3");
        assert_eq!(
            r.headers["x-ratelimit-requests-remaining"],
            remaining.to_string().as_str()
        );
        assert!(r.headers.contains_key("x-ratelimit-requests-reset"));
    }
    let r = send(&router, ping()).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.json()["error"], "rate_limited");
    let retry: u64 = r.headers["retry-after"].to_str().unwrap().parse().unwrap();
    assert!(retry >= 1);
    assert_eq!(r.headers["x-ratelimit-requests-remaining"], "0");
    // Health-Checks sind ausgenommen
    let health = Request::get("/healthz").body(Body::empty()).unwrap();
    assert_eq!(send(&router, health).await.status, StatusCode::OK);
    // Authentifizierte Identität hat einen eigenen Bucket
    let authed = Request::get("/ping")
        .header("authorization", KEY_RW)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, authed).await.status, StatusCode::OK);
}

#[tokio::test]
async fn complexity_budget_is_enforced_on_graphql() {
    let app = app_with_keys();
    let mut cfg = PlatformConfig::new(&app, false);
    cfg.rate.api_key = ClassLimits {
        requests: 1000.0,
        complexity: 3.0,
    };
    // Wie `router`, aber mit knappem Komplexitätsbudget.
    let router = apply_with(lineal_api::router::routes(app.clone(), false), app, cfg);
    let q = "{ __typename }"; // Komplexität 1
    for _ in 0..3 {
        let r = send(&router, gql(Some(("authorization", KEY_RW)), q)).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    }
    let r = send(&router, gql(Some(("authorization", KEY_RW)), q)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.json()["errors"][0]["extensions"]["code"], "rate_limited");
    assert!(r.headers.contains_key("retry-after"));
}

// ---------------------------------------------------------------- Security-Header, CORS

#[tokio::test]
async fn security_headers_are_present() {
    let router = api(app_with_keys());
    let r = send(
        &router,
        Request::get("/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    let csp = r.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'") && csp.contains("script-src 'self'"));
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    assert_eq!(
        r.headers["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
    assert!(
        !r.headers.contains_key("strict-transport-security"),
        "http: kein HSTS"
    );

    // HSTS bei https-Basis-URL
    let mut app = Arc::try_unwrap(app_with_keys()).ok().unwrap();
    app.config.base_url = "https://lineal.example".into();
    let r = send(
        &api(Arc::new(app)),
        Request::get("/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    assert!(
        r.headers["strict-transport-security"]
            .to_str()
            .unwrap()
            .contains("max-age=")
    );
}

#[tokio::test]
async fn cors_only_in_dev_and_only_for_the_vite_origin() {
    let preflight = |origin: &str| {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/graphql")
            .header("origin", origin)
            .header("access-control-request-method", "POST")
            .header(
                "access-control-request-headers",
                "content-type,authorization",
            )
            .body(Body::empty())
            .unwrap()
    };
    let dev = lineal_api::router(app_with_keys(), true);
    let r = send(&dev, preflight("http://localhost:5173")).await;
    assert_eq!(
        r.headers["access-control-allow-origin"],
        "http://localhost:5173"
    );
    assert_eq!(r.headers["access-control-allow-credentials"], "true");
    let r = send(&dev, preflight("http://evil.example")).await;
    assert!(!r.headers.contains_key("access-control-allow-origin"));
    let prod = api(app_with_keys());
    let r = send(&prod, preflight("http://localhost:5173")).await;
    assert!(!r.headers.contains_key("access-control-allow-origin"));
}

// ---------------------------------------------------------------- Blob-Store

struct At(DateTime<Utc>);

impl Clock for At {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lineal-blob-test-{}", uuid::Uuid::now_v7()));
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

fn blob_app(dir: &TempDir) -> Arc<App> {
    let mut app = Arc::try_unwrap(app_with_keys()).ok().unwrap();
    app.config.blob_dir = dir.0.clone();
    app.config.base_url = "http://lineal.test".into();
    Arc::new(app)
}

fn store_at(app: &App, ts: DateTime<Utc>) -> LocalBlobStore {
    LocalBlobStore::new(
        &app.config.blob_dir,
        &app.config.session_secret,
        &app.config.base_url,
        Arc::new(At(ts)),
    )
}

fn path_of(url: &str) -> String {
    url.trim_start_matches("http://lineal.test").to_owned()
}

fn put(path: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn upload_then_download_roundtrip() {
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let now = app.clock.now();
    let store = store_at(&app, now);
    let put_url = store
        .signed_put_url("acme/eng/diagram.png", "image/png", PUT_TTL)
        .await
        .unwrap();
    let r = send(&router, put(&path_of(&put_url), "image/png", PNG.to_vec())).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    let get_url = store
        .signed_get_url("acme/eng/diagram.png", GET_TTL)
        .await
        .unwrap();
    let res = router
        .clone()
        .oneshot(Request::get(path_of(&get_url)).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(
        res.headers()["content-disposition"],
        "inline; filename=\"diagram.png\""
    );
    assert_eq!(
        res.headers()["content-security-policy"],
        "default-src 'none'; sandbox"
    );
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], PNG);

    // unbekannter Schlüssel -> 404
    let other = store
        .signed_get_url("acme/none.txt", GET_TTL)
        .await
        .unwrap();
    let r = send(
        &router,
        Request::get(path_of(&other)).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn html_uploads_are_served_as_attachment() {
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let store = store_at(&app, app.clock.now());
    let url = store
        .signed_put_url("x/evil.html", "text/html", PUT_TTL)
        .await
        .unwrap();
    assert_eq!(
        send(
            &router,
            put(&path_of(&url), "text/html", b"<script>1</script>".to_vec())
        )
        .await
        .status,
        StatusCode::OK
    );
    let get = store.signed_get_url("x/evil.html", GET_TTL).await.unwrap();
    let res = router
        .oneshot(Request::get(path_of(&get)).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        res.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
}

#[tokio::test]
async fn expired_or_tampered_signatures_are_403() {
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let now = app.clock.now();
    // Signiert zwei Minuten in der Vergangenheit: Upload-URL (60 s) ist abgelaufen.
    let past = now - chrono::Duration::seconds(120);
    let old = store_at(&app, past)
        .signed_put_url("k.txt", "text/plain", PUT_TTL)
        .await
        .unwrap();
    let r = send(&router, put(&path_of(&old), "text/plain", b"x".to_vec())).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"], "forbidden");

    // Download-URL (5 min) ist nach 2 Minuten noch gültig, nach 6 nicht.
    let get_old = store_at(&app, now - chrono::Duration::seconds(360))
        .signed_get_url("k.txt", GET_TTL)
        .await
        .unwrap();
    let r = send(
        &router,
        Request::get(path_of(&get_old)).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Manipulierte Signatur
    let fresh = store_at(&app, now)
        .signed_put_url("k.txt", "text/plain", PUT_TTL)
        .await
        .unwrap();
    let mut forged = path_of(&fresh);
    forged.push('A');
    let r = send(&router, put(&forged, "text/plain", b"x".to_vec())).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Upload-Token darf nicht zum Download benutzt werden
    let as_get = path_of(&fresh).replace("/uploads/", "/files/");
    let r = send(&router, Request::get(as_get).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upload_checks_content_type_and_size() {
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let store = store_at(&app, app.clock.now());
    let url = store
        .signed_put_url("k.png", "image/png", PUT_TTL)
        .await
        .unwrap();
    // falscher Content-Type
    let r = send(&router, put(&path_of(&url), "text/html", vec![0])).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_request");
}

#[tokio::test]
async fn upload_over_the_reserved_size_is_rejected() {
    // B3.07: die in fileUpload reservierte Größe wird beim PUT erzwungen.
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let store = store_at(&app, app.clock.now());
    let url = store
        .signed_put_url_sized("k2.png", "image/png", 12, PUT_TTL)
        .await
        .unwrap();
    let mut big = PNG.to_vec();
    big.extend(vec![0u8; 100]);
    let r = send(&router, put(&path_of(&url), "image/png", big)).await;
    assert!(r.status.is_client_error(), "{}", r.body);
    assert_eq!(r.json()["error"], "limit_exceeded");
    // Genau in der Größe: ok.
    let r = send(&router, put(&path_of(&url), "image/png", PNG.to_vec())).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
}

#[tokio::test]
async fn upload_content_must_match_the_declared_image_type() {
    // B3.08: Magic-Byte-Prüfung.
    let dir = TempDir::new();
    let app = blob_app(&dir);
    let router = api(app.clone());
    let store = store_at(&app, app.clock.now());
    let url = store
        .signed_put_url("fake.png", "image/png", PUT_TTL)
        .await
        .unwrap();
    let r = send(
        &router,
        put(&path_of(&url), "image/png", b"not an image".to_vec()),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
    assert_eq!(r.json()["error"], "invalid_request");
    // Abgelehnte Datei wurde nicht abgelegt.
    assert!(store.head("fake.png").await.unwrap().is_none());
    // Text bleibt frei.
    let url = store
        .signed_put_url("a.txt", "text/plain", PUT_TTL)
        .await
        .unwrap();
    let r = send(
        &router,
        put(&path_of(&url), "text/plain", b"hello".to_vec()),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn oversized_graphql_body_is_413_with_consistent_error_and_byte_limit() {
    let router = api(app_with_keys());
    let huge = format!(
        r#"{{"query":"{{ __typename }}","pad":"{}"}}"#,
        "x".repeat(3 * 1024 * 1024)
    );
    let mut req = Request::post("/graphql")
        .header("content-type", "application/json")
        .header("authorization", KEY_RW)
        .body(Body::from(huge))
        .unwrap();
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        "203.0.113.60:4000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let r = send(&router, req).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", r.body);
    let ext = &r.json()["errors"][0]["extensions"];
    assert_eq!(ext["code"], "limit_exceeded");
    assert_eq!(ext["status"], 413);
    assert_eq!(ext["details"]["maxBytes"], 2 * 1024 * 1024);
    assert!(
        r.json()["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("bytes")
    );
}

#[tokio::test]
async fn schema_sdl_is_served_to_authenticated_clients_only() {
    let router = api(app_with_keys());
    let sdl = |auth: Option<&str>| {
        let mut req = from_peer("203.0.113.61", "");
        *req.uri_mut() = "/graphql/schema.graphql".parse().unwrap();
        if let Some(a) = auth {
            req.headers_mut()
                .insert("authorization", a.parse().unwrap());
        }
        req
    };
    assert_eq!(
        send(&router, sdl(None)).await.status,
        StatusCode::UNAUTHORIZED
    );
    let r = send(&router, sdl(Some(KEY_RW))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.contains("type Query"),
        "{}",
        &r.body[..200.min(r.body.len())]
    );
}

// ---------------------------------------------------------------- Brute-Force

#[tokio::test]
async fn failed_token_attempts_block_the_ip_independent_of_the_request_bucket() {
    let router = api(app_with_keys());
    let bad = |peer: &str, n: usize| {
        let mut req = from_peer(peer, "");
        *req.uri_mut() = "/graphql".parse().unwrap();
        *req.method_mut() = Method::POST;
        req.headers_mut()
            .insert("content-type", "application/json".parse().unwrap());
        req.headers_mut().insert(
            "authorization",
            format!("lin_api_guess{n}").parse().unwrap(),
        );
        *req.body_mut() = Body::from(r#"{"query":"{ __typename }"}"#);
        req
    };
    // 20 Fehlversuche mit jeweils anderem Token: 401, danach 429 (auch für ein gültiges Token).
    for n in 0..20 {
        let r = send(&router, bad("203.0.113.50", n)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "versuch {n}");
    }
    let r = send(&router, bad("203.0.113.50", 99)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.body);
    let mut valid = bad("203.0.113.50", 0);
    valid
        .headers_mut()
        .insert("authorization", KEY_RW.parse().unwrap());
    assert_eq!(
        send(&router, valid).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Eine andere IP ist nicht betroffen.
    assert_eq!(
        send(&router, bad("203.0.113.51", 0)).await.status,
        StatusCode::UNAUTHORIZED
    );
}
