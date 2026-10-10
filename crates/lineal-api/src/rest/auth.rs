//! `POST /auth/login`, `POST /auth/logout`, `GET /auth/me` (WP02).
//!
//! Login setzt das Session-Cookie `lineal_session` (HttpOnly, SameSite=Lax, bei https `Secure`);
//! das Token wird nur als SHA-256-Hash gespeichert. Die Handler lesen das Token selbst (Cookie oder
//! `Authorization`-Header); die zentrale Auth-Middleware (WP03) nutzt dieselbe Hash-Funktion.

use crate::auth_failures::AuthFailures;
use crate::client_ip::ClientIp;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lineal_app::usecases::auth::{LoginInput, SESSION_TTL_DAYS};
use lineal_app::{Actor, App, AppError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// Name des Session-Cookies.
pub const SESSION_COOKIE: &str = "lineal_session";

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
    /// `urlKey` des Workspace (nur nötig, wenn die E-Mail in mehreren Workspaces existiert).
    #[serde(default)]
    pub workspace: Option<String>,
}

fn error_response(e: AppError) -> Response {
    let status =
        StatusCode::from_u16(e.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = json!({
        "error": e.kind.code(),
        "message": e.message,
        "status": e.kind.http_status(),
        "requestId": "req_unknown",
        "details": e.details,
    });
    (status, Json(body)).into_response()
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for h in headers.get_all(header::COOKIE) {
        let Ok(s) = h.to_str() else { continue };
        for part in s.split(';') {
            if let Some((k, v)) = part.trim().split_once('=')
                && k == name
                && !v.is_empty()
            {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = v.strip_prefix("Bearer ").unwrap_or(v).trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn session_cookie(app: &App, token: &str, max_age_secs: i64) -> Option<HeaderValue> {
    let secure = if app.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}"
    ))
    .ok()
}

/// `POST /auth/login`: `{ "email", "password", "workspace"? }`.
pub async fn login(
    State(app): State<Arc<App>>,
    fails: Option<Extension<Arc<AuthFailures>>>,
    client: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    // Brute-Force-Schutz: je IP und je Ziel-Account, unabhängig vom normalen Request-Limit.
    let now_ms = app.clock.now().timestamp_millis();
    let ip_key = client.map(|Extension(c)| crate::rate_limit::ip_bucket(c.0));
    let account = body.email.clone();
    if let Some(Extension(f)) = &fails {
        let wait = ip_key
            .as_deref()
            .and_then(|ip| f.ip_blocked(ip, now_ms))
            .or_else(|| f.account_blocked(&account, now_ms));
        if let Some(wait) = wait {
            return crate::middleware::auth::blocked_response("/auth/login", wait);
        }
    }
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(300).collect());
    let result = app
        .login(LoginInput {
            email: body.email,
            password: body.password,
            workspace: body.workspace,
            user_agent,
            ip_address: None,
        })
        .await;
    match result {
        Ok(login) => {
            let mut res = (
                StatusCode::OK,
                Json(json!({
                    "userId": login.user_id.to_string(),
                    "organizationId": login.org_id.to_string(),
                    "expiresAt": login.expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    "csrfToken": login.csrf_token,
                })),
            )
                .into_response();
            if let Some(c) = session_cookie(&app, &login.token, SESSION_TTL_DAYS * 24 * 3600) {
                res.headers_mut().append(header::SET_COOKIE, c);
            }
            res
        }
        Err(e) => {
            if e.kind == lineal_app::ErrorKind::Unauthenticated
                && let Some(Extension(f)) = &fails
            {
                if let Some(ip) = &ip_key {
                    f.record_ip(ip, now_ms);
                }
                f.record_account(&account, now_ms);
            }
            error_response(e)
        }
    }
}

/// `POST /auth/logout`: widerruft die Session serverseitig und löscht das Cookie.
pub async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE)
        && let Err(e) = app.logout(&token).await
    {
        return error_response(e);
    }
    let mut res = (StatusCode::OK, Json(json!({ "success": true }))).into_response();
    if let Some(c) = session_cookie(&app, "", 0) {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}

/// `GET /auth/me`: der eingeloggte User (Session-Cookie oder `Authorization`-Header).
pub async fn me(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let Some(token) = cookie_value(&headers, SESSION_COOKIE).or_else(|| bearer(&headers)) else {
        return error_response(AppError::unauthenticated("Missing credentials."));
    };
    let principal = match app.principal_for_token(&token).await {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };
    let user_id = match principal.actor {
        Actor::User { id, .. } | Actor::App { id } => id,
        Actor::System => return error_response(AppError::unauthenticated("Invalid token.")),
    };
    let ctx = lineal_app::RequestCtx {
        org_id: principal.org_id,
        actor: principal.actor,
        request_id: "req_me".into(),
        scopes: principal.scopes,
    };
    let (user, org) = match (
        app.get_user(&ctx, &user_id.to_string()).await,
        app.get_workspace(&ctx).await,
    ) {
        (Ok(u), Ok(o)) => (u, o),
        (Err(e), _) | (_, Err(e)) => return error_response(e),
    };
    let body: Value = json!({
        "id": user.user.id.to_string(),
        "name": user.user.name,
        "displayName": user.user.display_name,
        "email": user.user.email,
        "role": user.user.role.as_str(),
        "workspace": { "id": org.id.to_string(), "name": org.name, "urlKey": org.url_key },
        "teams": user.teams.iter().map(|t| json!({
            "id": t.team.id.to_string(),
            "key": t.team.key,
            "name": t.team.name,
            "parentId": t.team.parent_id.map(|p| p.to_string()),
            "cyclesEnabled": t.team.cycles_enabled,
            "triageEnabled": t.team.triage_enabled,
        })).collect::<Vec<_>>(),
    });
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::router;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use lineal_app::ports::Store;
    use lineal_db::PgStore;
    use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
    use tower::ServiceExt;

    async fn setup(pool: sqlx::PgPool) -> Router {
        let store = PgStore::new(pool);
        let mut tx = store.begin(SeedData::acme().org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let app = test_app(Arc::new(store));
        // Seed-Hash ist Argon2; der Testkit-Hasher kennt nur `fake:`-Hashes -> Passwort setzen.
        app.set_user_password(&ctx_for(SeedUser::Alice), "me", "lineal-dev")
            .await
            .unwrap();
        router(app, false)
    }
    use axum::Router;

    async fn send(r: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let res = r.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn login_req(password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": "alice@acme.example", "password": password }).to_string(),
            ))
            .unwrap()
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn login_me_logout_roundtrip(pool: sqlx::PgPool) {
        let r = setup(pool).await;
        let (status, headers, body) = send(&r, login_req("lineal-dev")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["csrfToken"].is_string());
        let cookie = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cookie.starts_with("lineal_session=")
                && cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Lax")
        );
        let pair = cookie.split(';').next().unwrap().to_string();

        let me = Request::builder()
            .uri("/auth/me")
            .header("cookie", &pair)
            .body(Body::empty())
            .unwrap();
        let (status, _, body) = send(&r, me).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["email"], "alice@acme.example");
        assert_eq!(body["role"], "owner");
        assert_eq!(body["workspace"]["urlKey"], "acme");

        // Ohne Origin/CSRF-Header lehnt der Schutz für Cookie-Sessions ab.
        let blocked = Request::builder()
            .method("POST")
            .uri("/auth/logout")
            .header("cookie", &pair)
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&r, blocked).await.0, StatusCode::FORBIDDEN);
        let out = Request::builder()
            .method("POST")
            .uri("/auth/logout")
            .header("cookie", &pair)
            .header("origin", "http://localhost:8080")
            .header("x-lineal-csrf", "1")
            .body(Body::empty())
            .unwrap();
        let (status, headers, _) = send(&r, out).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        let me = Request::builder()
            .uri("/auth/me")
            .header("cookie", &pair)
            .body(Body::empty())
            .unwrap();
        let (status, _, body) = send(&r, me).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthenticated");
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn repeated_wrong_passwords_lock_the_account_even_for_the_right_one(pool: sqlx::PgPool) {
        let r = setup(pool).await;
        for i in 0..10 {
            let (status, _, _) = send(&r, login_req(&format!("wrong-{i}"))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "versuch {i}");
        }
        // Das richtige Passwort hilft jetzt nicht mehr: 429 mit Retry-After.
        let (status, headers, body) = send(&r, login_req("lineal-dev")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert!(headers.contains_key(header::RETRY_AFTER));
        assert!(headers.get(header::SET_COOKIE).is_none());
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn wrong_credentials_are_indistinguishable(pool: sqlx::PgPool) {
        let r = setup(pool).await;
        let (s1, h1, b1) = send(&r, login_req("wrong-password")).await;
        let unknown = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "email": "ghost@acme.example", "password": "wrong-password" }).to_string(),
            ))
            .unwrap();
        let (s2, _, b2) = send(&r, unknown).await;
        assert_eq!(
            (s1, &b1["error"], &b1["message"]),
            (s2, &b2["error"], &b2["message"])
        );
        assert_eq!(s1, StatusCode::UNAUTHORIZED);
        assert!(h1.get(header::SET_COOKIE).is_none());
        let me = Request::builder()
            .uri("/auth/me")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&r, me).await.0, StatusCode::UNAUTHORIZED);
    }
}
