//! OAuth-2.1-Authorization-Server (WP16, docs/09 §2.3, docs/07 §1.3): `/oauth/register` (DCR,
//! RFC 7591), `/oauth/authorize` (Consent über die Session), `/oauth/token`
//! (`authorization_code` + PKCE S256 Pflicht, `refresh_token` mit Rotation) und `/oauth/revoke`.
//!
//! Tokens: `lin_oauth_…` (Access, 24 h) und `lin_refresh_…`; gespeichert wird nur
//! `hash_token` (SHA-256, exakt wie `middleware::auth::hash_token`). Die Validierung der
//! Access-Tokens übernimmt der bestehende Bearer-Pfad (`find_principal_by_token_hash`).

use crate::middleware::auth::{AuthState, CredentialClass, MaybeAuth, hash_token};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Duration;
use hmac::{Hmac, Mac};
use lineal_app::ports::repos::NewEvent;
use lineal_app::ports::repos::webhook::{
    ClientRegistration, NewAuthCode, NewOauthToken, TokenRotation,
};
use lineal_app::{Actor, App, AppResult};
use lineal_domain::enums::UserRole;
use lineal_domain::events::{DomainEvent, EventOrigin};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Gültigkeit des Autorisierungscodes.
pub const CODE_TTL_MIN: i64 = 10;
/// Gültigkeit des Access-Tokens.
pub const ACCESS_TTL_SECS: i64 = 24 * 3600;
/// Gnadenfrist, in der das vorherige Refresh-Token noch gilt.
pub const REFRESH_GRACE_MIN: i64 = 30;
/// Gültigkeit des Consent-Nonce.
const CONSENT_TTL_MIN: i64 = 10;
const SCOPES: [&str; 3] = ["read", "write", "admin"];

fn random_hex() -> String {
    format!(
        "{}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `base64url(SHA-256(verifier))` (RFC 7636 S256).
pub fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Prüft den PKCE-Verifier gegen die Challenge (Länge 43–128, Zeichensatz RFC 7636, konstante Zeit).
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let ok_chars = verifier
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~'));
    (43..=128).contains(&verifier.len())
        && ok_chars
        && ct_eq(s256(verifier).as_bytes(), challenge.as_bytes())
}

fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn no_store(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    res
}

// ---------------------------------------------------------------------------
// Registrierung (RFC 7591)
// ---------------------------------------------------------------------------

/// Erlaubt `https://`, `http://` nur für Loopback (native Apps/MCP-Clients) und private Schemata
/// (`cursor://`, `vscode://`); nie `javascript:`/`data:`/`file:`, keine Userinfo, kein Fragment.
pub fn valid_redirect_uri(uri: &str) -> bool {
    if uri.is_empty() || uri.len() > 500 || uri.contains(char::is_whitespace) {
        return false;
    }
    let Ok(u) = url::Url::parse(uri) else {
        return false;
    };
    if u.fragment().is_some() || !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => u.host().is_some(),
        "http" => matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        "javascript" | "data" | "file" | "vbscript" | "about" | "blob" | "ftp" | "ws" | "wss" => {
            false
        }
        s => s.contains('.') || s.len() >= 3,
    }
}

fn oauth_error(status: StatusCode, code: &str, description: &str) -> Response {
    no_store(
        (
            status,
            Json(json!({ "error": code, "error_description": description })),
        )
            .into_response(),
    )
}

/// `POST /oauth/register`.
pub async fn register(State(app): State<Arc<App>>, body: Bytes) -> Response {
    let bad = |d: &str| oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", d);
    if body.len() > 16 * 1024 {
        return bad("request too large");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return bad("body must be JSON");
    };
    let uris: Vec<String> = v["redirect_uris"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if uris.is_empty()
        || uris.len() > 10
        || uris.len() != v["redirect_uris"].as_array().map_or(0, Vec::len)
    {
        return bad("redirect_uris must contain 1-10 strings");
    }
    if !uris.iter().all(|u| valid_redirect_uri(u)) {
        return bad("invalid redirect_uri (https, loopback http or a private scheme)");
    }
    if let Some(m) = v.get("token_endpoint_auth_method").and_then(Value::as_str)
        && m != "none"
    {
        return bad(
            "only token_endpoint_auth_method `none` (public clients with PKCE) is supported",
        );
    }
    if let Some(g) = v.get("grant_types").and_then(Value::as_array)
        && !g
            .iter()
            .all(|x| matches!(x.as_str(), Some("authorization_code" | "refresh_token")))
    {
        return bad("unsupported grant_types");
    }
    let name = v["client_name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("MCP client");
    if name.chars().count() > 200 {
        return bad("client_name too long");
    }
    let client = ClientRegistration {
        client_id: format!("lin_client_{}", Uuid::new_v4().simple()),
        client_name: name.to_string(),
        redirect_uris: uris,
    };
    let now = app.clock.now();
    let res: AppResult<()> = async {
        let mut tx = app.store.begin_system().await?;
        tx.webhooks().oauth_register_client(&client, now).await?;
        tx.commit().await
    }
    .await;
    if res.is_err() {
        return oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "registration failed",
        );
    }
    no_store(
        (
            StatusCode::CREATED,
            Json(json!({
                "client_id": client.client_id,
                "client_id_issued_at": now.timestamp(),
                "client_name": client.client_name,
                "redirect_uris": client.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            })),
        )
            .into_response(),
    )
}

// ---------------------------------------------------------------------------
// Autorisierung
// ---------------------------------------------------------------------------

fn error_page(msg: &str) -> Response {
    let mut res = (
        StatusCode::BAD_REQUEST,
        Html(format!(
            "<!doctype html><meta charset=utf-8><title>Authorization error</title><h1>Authorization error</h1><p>{}</p>",
            esc(msg)
        )),
    )
        .into_response();
    res.headers_mut()
        .insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    no_store(res)
}

/// Hängt Query-Parameter an die Redirect-URI an (bestehende bleiben erhalten).
fn redirect_with(uri: &str, params: &[(&str, &str)]) -> Response {
    let Ok(mut u) = url::Url::parse(uri) else {
        return error_page("invalid redirect_uri");
    };
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    no_store(Redirect::to(u.as_str()).into_response())
}

fn parse_scopes(raw: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = vec!["read".into()];
    for s in raw.split([' ', ',']).filter(|s| !s.is_empty()) {
        if !SCOPES.contains(&s) {
            return None;
        }
        if !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    Some(out)
}

fn consent_nonce(
    app: &App,
    ctx: &lineal_app::RequestCtx,
    p: &HashMap<String, String>,
    exp: i64,
) -> String {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    let user = match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => id.0.to_string(),
        Actor::System => String::new(),
    };
    let msg = format!(
        "consent|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        ctx.org_id.0,
        user,
        get("client_id"),
        get("redirect_uri"),
        get("scope"),
        get("state"),
        get("code_challenge"),
        get("code_challenge_method"),
        exp
    );
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(app.config.session_secret.as_bytes())
        .unwrap_or_else(|_| <Hmac<Sha256> as Mac>::new_from_slice(b"x").expect("hmac key"));
    mac.update(msg.as_bytes());
    format!("{exp}.{:x}", mac.finalize().into_bytes())
}

fn consent_valid(
    app: &App,
    ctx: &lineal_app::RequestCtx,
    p: &HashMap<String, String>,
    now: i64,
) -> bool {
    let Some(given) = p.get("consent") else {
        return false;
    };
    let Some((exp, _)) = given.split_once('.') else {
        return false;
    };
    let Ok(exp) = exp.parse::<i64>() else {
        return false;
    };
    exp >= now
        && exp <= now + CONSENT_TTL_MIN * 60 + 5
        && ct_eq(consent_nonce(app, ctx, p, exp).as_bytes(), given.as_bytes())
}

/// Zustimmungsseite: Client-Name ist vom Client frei gewählt und wird als unverifiziert
/// gekennzeichnet; maßgeblich ist der angezeigte Ziel-Host der Weiterleitung.
fn consent_page(
    client_name: &str,
    redirect_uri: &str,
    scopes: &[String],
    p: &HashMap<String, String>,
    nonce: &str,
) -> Response {
    let host = url::Url::parse(redirect_uri)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(str::to_owned)
                .or_else(|| Some(u.scheme().to_owned()))
        })
        .unwrap_or_default();
    let mut hidden = String::new();
    let mut keys: Vec<&String> = p
        .keys()
        .filter(|k| !matches!(k.as_str(), "consent" | "decision"))
        .collect();
    keys.sort();
    for k in keys {
        hidden.push_str(&format!(
            "<input type=hidden name=\"{}\" value=\"{}\">",
            esc(k),
            esc(&p[k])
        ));
    }
    hidden.push_str(&format!(
        "<input type=hidden name=\"consent\" value=\"{}\">",
        esc(nonce)
    ));
    let admin_warning = if scopes.iter().any(|s| s == "admin") {
        "<p><strong>Warning:</strong> the <code>admin</code> permission allows this application to change workspace settings.</p>"
    } else {
        ""
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Authorize application</title>\
         <h1>Authorize application</h1>\
         <p>An application calling itself <b>{name}</b> (name not verified by Lineal) requests access to your workspace.</p>\
         <p>After approval you will be sent to <b>{host}</b>. Only continue if you trust this site.</p>\
         <p>Permissions: <b>{scopes}</b></p>{admin_warning}\
         <form method=post action=\"/oauth/authorize\">{hidden}\
         <button type=submit name=decision value=allow>Allow</button> \
         <button type=submit name=decision value=deny>Deny</button></form>",
        name = esc(client_name),
        host = esc(&host),
        scopes = esc(&scopes.join(", ")),
    );
    let mut res = Html(html).into_response();
    res.headers_mut()
        .insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    res.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; form-action 'self'; frame-ancestors 'none'"),
    );
    no_store(res)
}

fn query_string(p: &HashMap<String, String>, extra: &[(&str, &str)]) -> String {
    let mut keys: Vec<&String> = p
        .keys()
        .filter(|k| !matches!(k.as_str(), "consent" | "decision"))
        .collect();
    keys.sort();
    let mut parts: Vec<String> = keys
        .iter()
        .map(|k| format!("{}={}", pct(k), pct(&p[*k])))
        .collect();
    for (k, v) in extra {
        parts.push(format!("{}={}", pct(k), pct(v)));
    }
    parts.join("&")
}

/// `GET /oauth/authorize`: prüft die Anfrage und zeigt die Zustimmungsseite. Ein Code wird
/// hier nie ausgestellt (Schutz vor Consent per Link/Prefetch).
pub async fn authorize(
    State(app): State<Arc<App>>,
    MaybeAuth(auth): MaybeAuth,
    Query(p): Query<HashMap<String, String>>,
) -> Response {
    authorize_inner(app, auth, p, false).await
}

/// `POST /oauth/authorize` (form-urlencoded): Bestätigung aus dem Zustimmungsformular.
/// Stellt den Code nur mit gültigem, an Session und Parameter gebundenem Consent-Nonce aus.
pub async fn authorize_post(
    State(app): State<Arc<App>>,
    MaybeAuth(auth): MaybeAuth,
    Form(p): Form<HashMap<String, String>>,
) -> Response {
    authorize_inner(app, auth, p, true).await
}

async fn authorize_inner(
    app: Arc<App>,
    auth: AuthState,
    p: HashMap<String, String>,
    is_post: bool,
) -> Response {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    // 1. Client und redirect_uri: bei Fehlern NIE weiterleiten.
    let client = {
        let Ok(mut tx) = app.store.begin_system().await else {
            return error_page("temporarily unavailable");
        };
        match tx.webhooks().oauth_find_client(get("client_id")).await {
            Ok(Some(c)) => c,
            _ => return error_page("unknown client_id"),
        }
    };
    let redirect_uri = get("redirect_uri");
    if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
        return error_page("redirect_uri is not registered for this client");
    }
    let state = get("state");
    let err = |code: &str, desc: &str| {
        let mut params = vec![("error", code), ("error_description", desc)];
        if !state.is_empty() {
            params.push(("state", state));
        }
        redirect_with(redirect_uri, &params)
    };
    if get("response_type") != "code" {
        return err("unsupported_response_type", "response_type must be code");
    }
    if get("code_challenge").is_empty() {
        return err("invalid_request", "code_challenge is required (PKCE)");
    }
    if get("code_challenge_method") != "S256" {
        return err("invalid_request", "code_challenge_method must be S256");
    }
    let challenge = get("code_challenge");
    if !(43..=128).contains(&challenge.len())
        || !challenge
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return err("invalid_request", "malformed code_challenge");
    }
    let Some(mut scopes) = parse_scopes(get("scope")) else {
        return err(
            "invalid_scope",
            "scope must be a subset of read, write, admin",
        );
    };
    // 2. Anmeldung: nur Cookie-Sessions dürfen zustimmen (nicht API-Keys/OAuth-Tokens).
    let ctx = match &auth {
        AuthState::Authenticated {
            ctx,
            class: CredentialClass::Session,
        } => ctx.clone(),
        _ => {
            let next = format!("/oauth/authorize?{}", query_string(&p, &[]));
            let login = format!(
                "{}/login?next={}",
                app.config.base_url.trim_end_matches('/'),
                pct(&next)
            );
            return no_store(Redirect::to(&login).into_response());
        }
    };
    let role = match ctx.actor {
        Actor::User { role, .. } => role,
        _ => return error_page("only users can authorize applications"),
    };
    // admin-Scope nur für Admins/Owner.
    if scopes.iter().any(|s| s == "admin") && !matches!(role, UserRole::Owner | UserRole::Admin) {
        scopes.retain(|s| s != "admin");
    }
    let now = app.clock.now();
    // 3. Consent: nur ein POST mit gültigem Nonce zählt als Zustimmung; sonst Formular zeigen.
    if !is_post || !consent_valid(&app, &ctx, &p, now.timestamp()) {
        let exp = now.timestamp() + CONSENT_TTL_MIN * 60;
        let nonce = consent_nonce(&app, &ctx, &p, exp);
        return consent_page(&client.client_name, redirect_uri, &scopes, &p, &nonce);
    }
    if get("decision") != "allow" {
        return err("access_denied", "the user denied the request");
    }
    // 4. Code ausstellen.
    let code = format!("lin_code_{}", random_hex());
    let Actor::User { id: user_id, .. } = ctx.actor else {
        return error_page("only users can authorize applications");
    };
    let res: AppResult<()> = async {
        let mut tx = app.store.begin(ctx.org_id).await?;
        let application = tx
            .webhooks()
            .oauth_ensure_application(&client, user_id, now)
            .await?;
        tx.webhooks()
            .oauth_insert_code(&NewAuthCode {
                code_hash: hash_token(&code),
                application_id: application,
                user_id,
                redirect_uri: redirect_uri.to_string(),
                scopes: scopes.clone(),
                code_challenge: challenge.to_string(),
                actor: "user".into(),
                family_id: Uuid::new_v4(),
                expires_at: now + Duration::minutes(CODE_TTL_MIN),
                now,
            })
            .await?;
        tx.commit().await
    }
    .await;
    if res.is_err() {
        return err("server_error", "could not issue the authorization code");
    }
    let mut params = vec![("code", code.as_str())];
    if !state.is_empty() {
        params.push(("state", state));
    }
    redirect_with(redirect_uri, &params)
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

fn invalid_grant(d: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", d)
}

fn token_response(access: &str, refresh: &str, scopes: &[String]) -> Response {
    no_store(
        Json(json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TTL_SECS,
            "scope": scopes.join(" "),
            "refresh_token": refresh,
        }))
        .into_response(),
    )
}

/// `POST /oauth/token` (form-urlencoded).
pub async fn token(
    State(app): State<Arc<App>>,
    Form(p): Form<HashMap<String, String>>,
) -> Response {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    match get("grant_type") {
        "authorization_code" => grant_code(&app, &p).await,
        "refresh_token" => grant_refresh(&app, &p).await,
        _ => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "supported: authorization_code, refresh_token",
        ),
    }
}

async fn grant_code(app: &App, p: &HashMap<String, String>) -> Response {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    let (code, verifier) = (get("code"), get("code_verifier"));
    if code.is_empty() || code.len() > 512 {
        return invalid_grant("unknown or invalid code");
    }
    if verifier.is_empty() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code_verifier is required (PKCE S256)",
        );
    }
    let hash = hash_token(code);
    let res: AppResult<Response> = async {
        let mut sys = app.store.begin_system().await?;
        let Some(rec) = sys.webhooks().oauth_find_code(&hash).await? else {
            return Ok(invalid_grant("unknown or invalid code"));
        };
        drop(sys);
        let now = app.clock.now();
        let mut tx = app.store.begin(rec.org_id).await?;
        // Atomar verbrauchen. Ein zweiter Versuch widerruft alle daraus entstandenen Tokens.
        if !tx.webhooks().oauth_consume_code(&hash, now).await? {
            tx.webhooks()
                .oauth_revoke_family(rec.family_id, now)
                .await?;
            tx.outbox()
                .push(NewEvent::new(
                    DomainEvent::OAuthAuthorizationRevoked {
                        client_id: rec.client_id.clone(),
                    },
                    EventOrigin::System,
                ))
                .await?;
            tx.commit().await?;
            return Ok(invalid_grant("authorization code already used"));
        }
        // Der Code ist verbraucht (auch bei Fehlern unten: kein Raten mit demselben Code).
        let checks = rec.expires_at > now
            && rec.client_id == get("client_id")
            && rec.redirect_uri == get("redirect_uri")
            && pkce_matches(verifier, &rec.code_challenge);
        if !checks {
            tx.commit().await?;
            return Ok(invalid_grant(
                "code, client_id, redirect_uri or code_verifier is invalid",
            ));
        }
        let (access, refresh) = (
            format!("lin_oauth_{}", random_hex()),
            format!("lin_refresh_{}", random_hex()),
        );
        tx.webhooks()
            .oauth_insert_token(&NewOauthToken {
                id: Uuid::new_v4(),
                application_id: rec.application_id,
                user_id: rec.user_id,
                actor: rec.actor.clone(),
                scopes: rec.scopes.clone(),
                access_token_hash: hash_token(&access),
                access_expires_at: now + Duration::seconds(ACCESS_TTL_SECS),
                refresh_token_hash: hash_token(&refresh),
                family_id: rec.family_id,
                now,
            })
            .await?;
        tx.commit().await?;
        Ok(token_response(&access, &refresh, &rec.scopes))
    }
    .await;
    res.unwrap_or_else(|_| {
        oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "token request failed",
        )
    })
}

async fn grant_refresh(app: &App, p: &HashMap<String, String>) -> Response {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    let presented = get("refresh_token");
    if presented.is_empty() || presented.len() > 512 {
        return invalid_grant("unknown refresh_token");
    }
    let hash = hash_token(presented);
    let res: AppResult<Response> = async {
        let mut sys = app.store.begin_system().await?;
        let Some(found) = sys.webhooks().oauth_find_by_refresh_hash(&hash).await? else {
            return Ok(invalid_grant("unknown refresh_token"));
        };
        drop(sys);
        let now = app.clock.now();
        let mut tx = app.store.begin(found.org_id).await?;
        let Some(rec) = tx.webhooks().oauth_find_by_refresh_hash(&hash).await? else {
            return Ok(invalid_grant("unknown refresh_token"));
        };
        if rec.revoked_at.is_some() || rec.client_id != get("client_id") {
            return Ok(invalid_grant(
                "refresh_token is revoked or belongs to another client",
            ));
        }
        let is_current = rec.refresh_token_hash.as_deref() == Some(hash.as_str());
        if !is_current {
            // Vorheriges Token: nur innerhalb der Gnadenfrist; danach Wiederverwendung = Diebstahl.
            let in_grace = rec
                .rotated_at
                .is_some_and(|at| now - at < Duration::minutes(REFRESH_GRACE_MIN));
            if !in_grace {
                tx.webhooks()
                    .oauth_revoke_family(rec.family_id, now)
                    .await?;
                tx.webhooks()
                    .disable_webhooks_of_application(rec.application_id, now)
                    .await?;
                tx.outbox()
                    .push(NewEvent::new(
                        DomainEvent::OAuthAuthorizationRevoked {
                            client_id: rec.client_id.clone(),
                        },
                        EventOrigin::System,
                    ))
                    .await?;
                tx.commit().await?;
                return Ok(invalid_grant(
                    "refresh_token was reused; the token family has been revoked",
                ));
            }
        }
        let current = rec.refresh_token_hash.clone().unwrap_or_default();
        let (access, refresh) = (
            format!("lin_oauth_{}", random_hex()),
            format!("lin_refresh_{}", random_hex()),
        );
        let rotated = tx
            .webhooks()
            .oauth_rotate_token(
                rec.id,
                &current,
                &TokenRotation {
                    access_token_hash: hash_token(&access),
                    access_expires_at: now + Duration::seconds(ACCESS_TTL_SECS),
                    refresh_token_hash: hash_token(&refresh),
                    now,
                },
            )
            .await?;
        if !rotated {
            return Ok(invalid_grant("refresh_token was rotated concurrently"));
        }
        tx.commit().await?;
        Ok(token_response(&access, &refresh, &rec.scopes))
    }
    .await;
    res.unwrap_or_else(|_| {
        oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "token request failed",
        )
    })
}

/// `POST /oauth/revoke` (RFC 7009): widerruft die Token-Familie; antwortet immer 200.
pub async fn revoke(
    State(app): State<Arc<App>>,
    Form(p): Form<HashMap<String, String>>,
) -> Response {
    let token = p.get("token").map(String::as_str).unwrap_or("");
    if !token.is_empty() && token.len() <= 512 {
        let hash = hash_token(token);
        let now = app.clock.now();
        let res: AppResult<()> = async {
            let mut tx = app.store.begin_system().await?;
            let hit = tx.webhooks().oauth_revoke_by_token_hash(&hash, now).await?;
            tx.commit().await?;
            if let Some((org, client_id)) = hit {
                let mut tx = app.store.begin(org).await?;
                tx.outbox()
                    .push(NewEvent::new(
                        DomainEvent::OAuthAuthorizationRevoked { client_id },
                        EventOrigin::System,
                    ))
                    .await?;
                tx.commit().await?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "oauth revoke failed");
        }
    }
    no_store(StatusCode::OK.into_response())
}
