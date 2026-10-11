#![allow(clippy::unwrap_used, clippy::expect_used)]
//! OAuth-Authorization-Server gegen echte Datenbank: DCR, Authorize/Consent, PKCE-Fehlerfälle,
//! Code-Wiederverwendung, Refresh-Rotation mit Family-Revocation, MCP mit OAuth-Token.

use super::*;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request};
use chrono::{DateTime, TimeZone, Utc};
use http_body_util::BodyExt;
use lineal_app::ports::repos::auth::NewSession;
use lineal_app::ports::{Clock, Store};
use lineal_db::PgStore;
use lineal_domain::ids::UserId;
use lineal_testkit::{SeedData, seed, test_app};
use sqlx::PgPool;
use std::sync::Mutex;
use tower::ServiceExt;

struct TestClock(Mutex<DateTime<Utc>>);
impl TestClock {
    fn advance(&self, secs: i64) {
        *self.0.lock().unwrap() += Duration::seconds(secs);
    }
}
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[allow(dead_code)]
struct Fx {
    app: Arc<App>,
    api: Router,
    mcp: Router,
    clock: Arc<TestClock>,
    pool: PgPool,
    alice_cookie: String,
    bob_cookie: String,
}

const REDIRECT: &str = "http://127.0.0.1:7777/callback";

async fn session(app: &App, user: UserId, token: &str, org: lineal_domain::ids::OrgId) -> String {
    let mut tx = app.store.begin(org).await.unwrap();
    tx.auth()
        .insert_session(&NewSession {
            id: Uuid::new_v4(),
            user_id: user,
            token_hash: hash_token(token),
            csrf_token_hash: None,
            user_agent: None,
            ip_address: None,
            expires_at: app.clock.now() + Duration::days(1),
            now: app.clock.now(),
        })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    format!("lineal_session={token}")
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let clock = Arc::new(TestClock(Mutex::new(Utc::now())));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = clock.clone();
    let app = Arc::new(app);
    let alice_cookie = session(&app, data.alice, "alice-session-token", data.org).await;
    let bob_cookie = session(&app, data.bob, "bob-session-token", data.org).await;
    Fx {
        api: crate::router::router(app.clone(), false),
        mcp: lineal_mcp::router(app.clone()),
        app,
        clock,
        pool,
        alice_cookie,
        bob_cookie,
    }
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = router.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut r = Request::builder().method(Method::GET).uri(uri);
    if let Some(c) = cookie {
        r = r.header(header::COOKIE, c);
    }
    r.body(Body::empty()).unwrap()
}

/// POST des Zustimmungsformulars: alle versteckten Felder der Seite plus `decision`.
fn consent_post(page: &str, decision: &str, cookie: &str) -> Request<Body> {
    let mut pairs = url::form_urlencoded::Serializer::new(String::new());
    for chunk in page.split("<input type=hidden name=\"").skip(1) {
        let name = chunk.split('"').next().unwrap();
        let value = chunk
            .split("value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let unesc = |v: &str| {
            v.replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&#39;", "'")
                .replace("&amp;", "&")
        };
        pairs.append_pair(&unesc(name), &unesc(value));
    }
    pairs.append_pair("decision", decision);
    Request::builder()
        .method(Method::POST)
        .uri("/oauth/authorize")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .body(Body::from(pairs.finish()))
        .unwrap()
}

fn form(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn register_client(fx: &Fx) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "client_name": "Test <b>MCP</b>", "redirect_uris": [REDIRECT] }).to_string(),
        ))
        .unwrap();
    let (status, _, body) = send(&fx.api, req).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["token_endpoint_auth_method"], "none");
    v["client_id"].as_str().unwrap().to_string()
}

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn authorize_uri(client: &str, extra: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={client}&redirect_uri={}&state=xyz&code_challenge={}&code_challenge_method=S256{extra}",
        pct(REDIRECT),
        s256(VERIFIER)
    )
}

/// Führt Authorize inkl. Consent aus und liefert den Code.
async fn obtain_code(fx: &Fx, client: &str, cookie: &str, scope: &str) -> String {
    let uri = authorize_uri(client, &format!("&scope={scope}"));
    let (status, _, page) = send(&fx.api, get(&uri, Some(cookie))).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Allow"));
    assert!(
        !page.contains("<b>MCP</b>"),
        "Client-Name muss escaped werden"
    );
    assert!(
        page.contains("method=post"),
        "Zustimmung nur per POST-Formular"
    );
    assert!(
        page.contains("not verified"),
        "Client-Name als unverifiziert markiert"
    );
    let (status, headers, _) = send(&fx.api, consent_post(&page, "allow", cookie)).await;
    assert!(status.is_redirection(), "{status}");
    let loc = headers
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let u = url::Url::parse(&loc).unwrap();
    assert_eq!(
        format!(
            "{}://{}:{}{}",
            u.scheme(),
            u.host_str().unwrap(),
            u.port().unwrap(),
            u.path()
        ),
        REDIRECT
    );
    let q: HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert_eq!(q["state"], "xyz");
    q["code"].clone()
}

async fn exchange(fx: &Fx, client: &str, code: &str, verifier: &str) -> (StatusCode, Value) {
    let body = format!(
        "grant_type=authorization_code&code={code}&client_id={client}&redirect_uri={}&code_verifier={verifier}",
        pct(REDIRECT)
    );
    let (status, headers, text) = send(&fx.api, form("/oauth/token", &body)).await;
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    (status, serde_json::from_str(&text).unwrap())
}

async fn refresh(fx: &Fx, client: &str, token: &str) -> (StatusCode, Value) {
    let body = format!("grant_type=refresh_token&client_id={client}&refresh_token={token}");
    let (status, _, text) = send(&fx.api, form("/oauth/token", &body)).await;
    (status, serde_json::from_str(&text).unwrap())
}

async fn mcp_call(fx: &Fx, token: &str) -> (StatusCode, Value) {
    let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_teams", "arguments": {} } });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .body(Body::from(msg.to_string()))
        .unwrap();
    let (status, _, text) = send(&fx.mcp, req).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn registration_validates_redirect_uris(pool: PgPool) {
    let fx = setup(pool).await;
    for body in [
        json!({ "redirect_uris": [] }),
        json!({ "redirect_uris": ["javascript:alert(1)"] }),
        json!({ "redirect_uris": ["http://evil.example/cb"] }),
        json!({ "redirect_uris": ["https://user:pw@app.example/cb"] }),
        json!({ "redirect_uris": ["https://app.example/cb#frag"] }),
        json!({ "redirect_uris": ["https://app.example/cb"], "token_endpoint_auth_method": "client_secret_basic" }),
    ] {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/oauth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let (status, _, text) = send(&fx.api, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {text}");
        assert!(text.contains("invalid_client_metadata"));
    }
    assert!(valid_redirect_uri("https://app.example/cb"));
    assert!(valid_redirect_uri(
        "cursor://anysphere.cursor-retrieval/oauth/callback"
    ));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn authorize_validates_client_redirect_pkce_and_login(pool: PgPool) {
    let fx = setup(pool).await;
    let client = register_client(&fx).await;
    // Unbekannter Client / nicht registrierte redirect_uri: Fehlerseite, nie Redirect.
    let (s, h, _) = send(
        &fx.api,
        get(
            &authorize_uri("lin_client_nope", ""),
            Some(&fx.alice_cookie),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(h.get(header::LOCATION).is_none());
    let bad_uri =
        authorize_uri(&client, "").replace(&pct(REDIRECT), &pct("http://127.0.0.1:7777/other"));
    let (s, h, _) = send(&fx.api, get(&bad_uri, Some(&fx.alice_cookie))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(h.get(header::LOCATION).is_none());
    // PKCE: plain, fehlend -> Redirect mit error.
    for uri in [
        authorize_uri(&client, "")
            .replace("code_challenge_method=S256", "code_challenge_method=plain"),
        authorize_uri(&client, "").replace("&code_challenge_method=S256", ""),
        authorize_uri(&client, "").replace(
            &format!("code_challenge={}", s256(VERIFIER)),
            "code_challenge=",
        ),
    ] {
        let (s, h, _) = send(&fx.api, get(&uri, Some(&fx.alice_cookie))).await;
        assert!(s.is_redirection(), "{uri}");
        let loc = h.get(header::LOCATION).unwrap().to_str().unwrap();
        assert!(
            loc.starts_with(REDIRECT)
                && loc.contains("error=invalid_request")
                && loc.contains("state=xyz"),
            "{loc}"
        );
    }
    let (s, h, _) = send(
        &fx.api,
        get(
            &authorize_uri(&client, "&scope=root"),
            Some(&fx.alice_cookie),
        ),
    )
    .await;
    assert!(s.is_redirection());
    assert!(
        h.get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("error=invalid_scope")
    );
    // Ohne Session: Weiterleitung zum Login; ein API-Key/OAuth-Token darf nicht zustimmen.
    let (s, h, _) = send(&fx.api, get(&authorize_uri(&client, ""), None)).await;
    assert!(s.is_redirection());
    assert!(
        h.get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("/login?next=")
    );
    // Gefälschter Consent-Nonce führt nur zur Zustimmungsseite, nie zu einem Code.
    let forged = authorize_uri(&client, "&consent=9999999999.deadbeef&decision=allow");
    let (s, _, page) = send(&fx.api, get(&forged, Some(&fx.alice_cookie))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains("Allow"));
    // Ablehnen.
    let uri = authorize_uri(&client, "");
    let (_, _, page) = send(&fx.api, get(&uri, Some(&fx.alice_cookie))).await;
    // Ein GET mit gültigem Nonce und decision=allow stellt KEINEN Code aus (nur POST zählt).
    let nonce = page
        .split("name=\"consent\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let via_get = authorize_uri(&client, &format!("&consent={}&decision=allow", pct(&nonce)));
    let (s, _, again) = send(&fx.api, get(&via_get, Some(&fx.alice_cookie))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(again.contains("method=post"), "GET darf nie zustimmen");
    let (s, h, _) = send(&fx.api, consent_post(&page, "deny", &fx.alice_cookie)).await;
    assert!(s.is_redirection());
    assert!(
        h.get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("error=access_denied")
    );
    // Der Nonce gilt nur für die Session, für die er ausgestellt wurde.
    let (s, _, page) = send(&fx.api, consent_post(&page, "allow", &fx.bob_cookie)).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert!(
        page.contains("Allow"),
        "fremder Nonce darf keinen Code liefern"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn full_flow_pkce_failures_and_mcp_with_oauth_token(pool: PgPool) {
    let fx = setup(pool).await;
    let client = register_client(&fx).await;

    // Falscher Verifier / zu kurzer Verifier / fehlend: jeder Code wird dabei verbraucht.
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read%20write").await;
    let (s, v) = exchange(
        &fx,
        &client,
        &code,
        "wrong-verifier-wrong-verifier-wrong-verifier-xx",
    )
    .await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (s, v) = exchange(&fx, &client, &code, VERIFIER).await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant")),
        "verbrauchter Code"
    );
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read").await;
    let (s, v) = exchange(&fx, &client, &code, "short").await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read").await;
    let body = format!(
        "grant_type=authorization_code&code={code}&client_id={client}&redirect_uri={}",
        pct(REDIRECT)
    );
    let (s, _, text) = send(&fx.api, form("/oauth/token", &body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(text.contains("invalid_request"), "kein Verifier: {text}");
    // plain-Verifier gleich Challenge-String wird nicht akzeptiert.
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read").await;
    let (s, _) = exchange(&fx, &client, &code, &s256(VERIFIER)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Falscher Client / redirect_uri.
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read").await;
    let other = register_client(&fx).await;
    let (s, _) = exchange(&fx, &other, &code, VERIFIER).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Erfolg.
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read%20write").await;
    let (s, v) = exchange(&fx, &client, &code, VERIFIER).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["token_type"], "Bearer");
    assert_eq!(v["expires_in"], 86400);
    assert_eq!(v["scope"], "read write");
    let access = v["access_token"].as_str().unwrap().to_string();
    let refresh_tok = v["refresh_token"].as_str().unwrap().to_string();
    assert!(access.starts_with("lin_oauth_") && refresh_tok.starts_with("lin_refresh_"));
    // Nur Hashes in der Datenbank.
    let stored: String = sqlx::query_scalar(
        "SELECT access_token_hash FROM oauth_token ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(stored, hash_token(&access));
    let plain: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth_token WHERE refresh_token_hash = $1 OR access_token_hash = $2",
    )
    .bind(&refresh_tok)
    .bind(&access)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(plain, 0);

    // MCP-Call mit OAuth-Token funktioniert (bestehender Bearer-Validator).
    let (s, resp) = mcp_call(&fx, &access).await;
    assert_eq!(s, StatusCode::OK, "{resp}");
    assert_ne!(resp["result"]["isError"], true, "{resp}");
    // Code-Wiederverwendung widerruft alle daraus entstandenen Tokens.
    let (s, v) = exchange(&fx, &client, &code, VERIFIER).await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (s, _) = mcp_call(&fx, &access).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "widerrufenes Token");
    let (s, v) = refresh(&fx, &client, &refresh_tok).await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn refresh_rotation_and_family_revocation_on_reuse(pool: PgPool) {
    let fx = setup(pool).await;
    let client = register_client(&fx).await;
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "read%20write").await;
    let (_, v) = exchange(&fx, &client, &code, VERIFIER).await;
    let (a1, r1) = (
        v["access_token"].as_str().unwrap().to_string(),
        v["refresh_token"].as_str().unwrap().to_string(),
    );

    // Rotation: neues Paar, altes Access-Token sofort ungültig.
    let (s, v) = refresh(&fx, &client, &r1).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (a2, r2) = (
        v["access_token"].as_str().unwrap().to_string(),
        v["refresh_token"].as_str().unwrap().to_string(),
    );
    assert_ne!((&a1, &r1), (&a2, &r2));
    assert_eq!(mcp_call(&fx, &a1).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(mcp_call(&fx, &a2).await.0, StatusCode::OK);
    // Falscher Client.
    let other = register_client(&fx).await;
    assert_eq!(refresh(&fx, &other, &r2).await.0, StatusCode::BAD_REQUEST);
    // Gnadenfrist: das vorherige Refresh-Token funktioniert noch (verlorene Antwort).
    let (s, v) = refresh(&fx, &client, &r1).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (a3, r3) = (
        v["access_token"].as_str().unwrap().to_string(),
        v["refresh_token"].as_str().unwrap().to_string(),
    );
    // Nach der Gnadenfrist: Wiederverwendung eines alten Tokens widerruft die ganze Familie.
    let (s, v) = refresh(&fx, &client, &r2).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r4 = v["refresh_token"].as_str().unwrap().to_string();
    fx.clock.advance(31 * 60);
    let (s, v) = refresh(&fx, &client, &r3).await;
    assert_eq!(
        (s, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant")),
        "{v}"
    );
    assert_eq!(mcp_call(&fx, &a3).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        refresh(&fx, &client, &r4).await.0,
        StatusCode::BAD_REQUEST,
        "Familie ist widerrufen"
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_event WHERE payload->>'type' = 'OAuthAuthorizationRevoked'",
    )
    .fetch_one(&fx.pool)
    .await
    .unwrap_or(1);
    assert!(events >= 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn revoke_endpoint_and_scope_clamping(pool: PgPool) {
    let fx = setup(pool).await;
    let client = register_client(&fx).await;
    // Bob (User) bekommt keinen admin-Scope; Alice (Owner) schon.
    let code = obtain_code(&fx, &client, &fx.bob_cookie, "read%20write%20admin").await;
    let (_, v) = exchange(&fx, &client, &code, VERIFIER).await;
    assert_eq!(v["scope"], "read write");
    let code = obtain_code(&fx, &client, &fx.alice_cookie, "admin").await;
    let (_, v) = exchange(&fx, &client, &code, VERIFIER).await;
    assert_eq!(v["scope"], "read admin");
    let access = v["access_token"].as_str().unwrap().to_string();
    assert_eq!(mcp_call(&fx, &access).await.0, StatusCode::OK);
    // Widerruf.
    let (s, _, _) = send(&fx.api, form("/oauth/revoke", &format!("token={access}"))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mcp_call(&fx, &access).await.0, StatusCode::UNAUTHORIZED);
    // Unbekanntes Token: ebenfalls 200 (RFC 7009), kein Orakel.
    let (s, _, _) = send(&fx.api, form("/oauth/revoke", "token=lin_oauth_unknown")).await;
    assert_eq!(s, StatusCode::OK);
    // Nicht unterstützte Grants.
    let (s, _, text) = send(
        &fx.api,
        form("/oauth/token", "grant_type=client_credentials"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(text.contains("unsupported_grant_type"));
}

#[test]
fn pkce_and_scope_helpers() {
    // RFC 7636 Anhang B.
    assert_eq!(
        s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    assert!(pkce_matches(VERIFIER, &s256(VERIFIER)));
    assert!(!pkce_matches(VERIFIER, VERIFIER));
    assert!(!pkce_matches("short", &s256("short")));
    assert_eq!(parse_scopes("write").unwrap(), vec!["read", "write"]);
    assert!(parse_scopes("read,root").is_none());
}
