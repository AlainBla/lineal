//! `POST /auth/login`, `POST /auth/logout`, `GET /auth/me` (WP02).
//!
//! Login setzt das Session-Cookie `lineal_session` (HttpOnly, SameSite=Lax, bei https `Secure`);
//! das Token wird nur als SHA-256-Hash gespeichert. Die Handler lesen das Token selbst (Cookie oder
//! `Authorization`-Header); die zentrale Auth-Middleware (WP03) nutzt dieselbe Hash-Funktion.

use crate::auth_failures::AuthFailures;
use crate::client_ip::ClientIp;
use crate::middleware::request_id::RequestId;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lineal_app::usecases::auth::{LoginInput, LoginOutcome, SESSION_TTL_DAYS};
use lineal_app::usecases::invitation::AcceptInvitationInput;
use lineal_app::usecases::totp::pending::{PendingScope, mint as mint_pending};
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

pub(crate) fn error_response(e: AppError) -> Response {
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

pub(crate) fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = v.strip_prefix("Bearer ").unwrap_or(v).trim();
    (!token.is_empty()).then(|| token.to_string())
}

pub(crate) fn session_cookie(app: &App, token: &str, max_age_secs: i64) -> Option<HeaderValue> {
    let secure = app.config.base_url.starts_with("https://");
    let name = crate::middleware::auth::session_cookie_name(secure);
    let secure_attr = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure_attr}"
    ))
    .ok()
}

/// Session-Cookie lesen und dabei beide Namensvarianten akzeptieren: in Produktion schickt der
/// Browser `__Host-lineal_session`, in der http-Entwicklung `lineal_session` (LIN-62 L-7).
pub(crate) fn session_cookie_value(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, crate::middleware::auth::SESSION_COOKIE_HOST)
        .or_else(|| cookie_value(headers, SESSION_COOKIE))
}

/// `POST /auth/login`: `{ "email", "password", "workspace"? }`.
pub async fn login(
    State(app): State<Arc<App>>,
    fails: Option<Extension<Arc<AuthFailures>>>,
    client: Option<Extension<ClientIp>>,
    request_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    // Brute-Force-Schutz: Zähler je IP, je Paar (Account, IP) und je Account werden VOR der
    // Passwortprüfung atomar reserviert und bei Erfolg zurückgegeben.
    let now_ms = app.clock.now().timestamp_millis();
    let client_ip = client.as_ref().map(|Extension(c)| c.0.to_string());
    let ip_key = client.map(|Extension(c)| crate::rate_limit::ip_bucket(c.0));
    // S-23: Ist das Account-Gesamtlimit erreicht (verteilter Angriff), würde es den Account auch
    // von der eigenen IP sperren. Nur dann die DB nach einer bekannten IP fragen (jüngst
    // erfolgreicher Login) – diese IP darf das Gesamtlimit passieren. Der Peek vermeidet eine
    // DB-Abfrage bei jedem normalen Login.
    let ip_known = match &fails {
        Some(Extension(f)) if f.account_total_blocked(&body.email, now_ms) => {
            app.login_ip_known(
                &body.email,
                client_ip.as_deref(),
                body.workspace.as_deref(),
                app.clock.now(),
            )
            .await
        }
        _ => false,
    };
    let reservation = match &fails {
        Some(Extension(f)) => {
            match f.reserve_login(&body.email, ip_key.as_deref(), ip_known, now_ms) {
                Ok(r) => Some(r),
                Err(wait) => {
                    return crate::middleware::auth::blocked_response("/auth/login", wait);
                }
            }
        }
        None => None,
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(300).collect());
    let result = app
        .login_outcome(LoginInput {
            email: body.email,
            password: body.password,
            workspace: body.workspace,
            user_agent,
            ip_address: client_ip,
            request_id: request_id.map(|Extension(r)| r.0),
        })
        .await;
    // Reservierung bei allem ausser falschen Zugangsdaten freigeben.
    let release = |reservation: Option<crate::auth_failures::LoginReservation>| {
        if let (Some(r), Some(Extension(f))) = (reservation, &fails) {
            r.release(f);
        }
    };
    match result {
        Ok(LoginOutcome::Session(login)) => {
            release(reservation);
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
        // Passwort stimmte: Reservierung freigeben, zweiter Schritt folgt per Pending-Token.
        Ok(LoginOutcome::TotpRequired { user_id, org_id }) => {
            release(reservation);
            pending_response(&app, user_id, org_id, PendingScope::Login, "totpRequired")
        }
        Ok(LoginOutcome::EnrollRequired { user_id, org_id }) => {
            release(reservation);
            pending_response(
                &app,
                user_id,
                org_id,
                PendingScope::Enroll,
                "enrollRequired",
            )
        }
        Err(e) => {
            // Falsche Zugangsdaten zählen (Reservierung bleibt); andere Fehler nicht.
            if e.kind != lineal_app::ErrorKind::Unauthenticated {
                release(reservation);
            }
            error_response(e)
        }
    }
}

/// Antwort für einen fälligen zweiten Schritt: stellt den Pending-Token aus.
fn pending_response(
    app: &App,
    user_id: lineal_domain::ids::UserId,
    org_id: lineal_domain::ids::OrgId,
    scope: PendingScope,
    flag: &str,
) -> Response {
    match mint_pending(
        &app.config.session_secret,
        user_id,
        org_id,
        scope,
        app.clock.now(),
    ) {
        Ok(token) => {
            let mut obj = serde_json::Map::new();
            obj.insert(flag.to_string(), json!(true));
            obj.insert("pendingToken".to_string(), json!(token));
            (StatusCode::OK, Json(Value::Object(obj))).into_response()
        }
        Err(e) => error_response(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct AcceptInviteBody {
    /// Token aus dem Einladungslink (`/invite/<token>`).
    pub token: String,
    /// Adresse, an die die Einladung ging.
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    pub password: String,
}

/// `POST /invite/accept`: nimmt eine Einladung ohne bestehende Session an (Token + E-Mail + Name +
/// Passwort), legt den User an bzw. verknüpft den Platzhalter und setzt das Session-Cookie.
/// Unbekannte, abgelaufene oder bereits angenommene Tokens antworten einheitlich mit `404`.
pub async fn accept_invite(
    State(app): State<Arc<App>>,
    Json(body): Json<AcceptInviteBody>,
) -> Response {
    let result = app
        .accept_invitation(AcceptInvitationInput {
            token: body.token,
            email: body.email,
            name: body.name,
            password: Some(body.password),
        })
        .await;
    match result {
        Ok(accepted) => {
            let Some(login) = accepted.login else {
                return error_response(AppError::internal("invitation accepted without session"));
            };
            let mut res = (
                StatusCode::OK,
                Json(json!({
                    "userId": accepted.user_id.to_string(),
                    "organizationId": accepted.org_id.to_string(),
                    "workspace": accepted.url_key,
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
        Err(e) => error_response(e),
    }
}

/// `POST /auth/logout`: widerruft die Session serverseitig und löscht das Cookie.
pub async fn logout(
    State(app): State<Arc<App>>,
    client: Option<Extension<ClientIp>>,
    request_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
) -> Response {
    let client_ip = client.map(|Extension(c)| c.0.to_string());
    let request_id = request_id.map(|Extension(r)| r.0);
    if let Some(token) = session_cookie_value(&headers)
        && let Err(e) = app.logout(&token, client_ip, request_id).await
    {
        return error_response(e);
    }
    let mut res = (StatusCode::OK, Json(json!({ "success": true }))).into_response();
    if let Some(c) = session_cookie(&app, "", 0) {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordBody {
    pub email: String,
    /// `urlKey` des Workspace; nötig, wenn die E-Mail in mehreren Workspaces existiert (wie Login).
    #[serde(default, rename = "urlKey", alias = "workspace")]
    pub url_key: Option<String>,
}

/// Neutrale Antwort auf jede Reset-Anfrage – identisch, ob ein Konto existiert, nicht existiert oder
/// das Limit erreicht ist (keine Enumeration über Status, Body oder Header).
fn forgot_password_ok() -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "message": "If an account exists for that email, we've sent a password reset link."
        })),
    )
        .into_response()
}

/// `POST /auth/forgot-password`: `{ "email", "urlKey"? }`. Antwortet IMMER mit demselben `200`.
///
/// Enumeration-/Timing-Schutz: Der Handler macht nur die schnelle In-Memory-Ratenreservierung und
/// stößt die gesamte eigentliche Arbeit (Lookup, Schreiben, Mailversand) in einer losgelösten Task
/// an — die Antwortzeit hängt damit nicht von der Kontoexistenz ab. Bei erreichtem Limit wird nichts
/// angestoßen, aber dieselbe Antwort geliefert.
pub async fn forgot_password(
    State(app): State<Arc<App>>,
    fails: Option<Extension<Arc<AuthFailures>>>,
    client: Option<Extension<ClientIp>>,
    Json(body): Json<ForgotPasswordBody>,
) -> Response {
    let now_ms = app.clock.now().timestamp_millis();
    let client_ip = client.as_ref().map(|Extension(c)| c.0.to_string());
    let ip_key = client.map(|Extension(c)| crate::rate_limit::ip_bucket(c.0));
    let allowed = match &fails {
        Some(Extension(f)) => f
            .reserve_password_reset(body.email.trim(), ip_key.as_deref(), now_ms)
            .is_ok(),
        None => true,
    };
    if allowed {
        let app = app.clone();
        let email = body.email.clone();
        let url_key = body.url_key.clone();
        let ip = client_ip.clone();
        tokio::spawn(async move {
            if let Err(e) = app
                .deliver_password_reset(&email, url_key.as_deref(), ip.as_deref())
                .await
            {
                tracing::warn!(error = %e, "password reset delivery failed");
            }
        });
    }
    forgot_password_ok()
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordBody {
    pub token: String,
    #[serde(rename = "newPassword", alias = "new_password")]
    pub new_password: String,
}

/// `POST /auth/reset-password`: `{ "token", "newPassword" }`. Setzt KEINE Session (frischer Login).
/// Unbekannte, abgelaufene oder verbrauchte Tokens antworten einheitlich neutral.
pub async fn reset_password(
    State(app): State<Arc<App>>,
    client: Option<Extension<ClientIp>>,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<ResetPasswordBody>,
) -> Response {
    let client_ip = client.map(|Extension(c)| c.0.to_string());
    let request_id = request_id.map(|Extension(r)| r.0);
    match app
        .reset_password(&body.token, &body.new_password, client_ip, request_id)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({ "success": true }))).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /auth/me`: der eingeloggte User (Session-Cookie oder `Authorization`-Header).
pub async fn me(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let cookie = session_cookie_value(&headers);
    let from_cookie = cookie.is_some();
    let Some(token) = cookie.or_else(|| bearer(&headers)) else {
        return error_response(AppError::unauthenticated("Missing credentials."));
    };
    let principal = match app.principal_for_token(&token).await {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };
    // Cookie nur für Sessions, Header nie für Sessions (kein Token-Typ-Mix).
    if from_cookie != (principal.kind == lineal_app::ports::repos::CredentialKind::Session) {
        return error_response(AppError::unauthenticated("Invalid or expired token."));
    }
    let user_id = match principal.actor {
        Actor::User { id, .. } | Actor::App { id } => id,
        Actor::System => return error_response(AppError::unauthenticated("Invalid token.")),
    };
    let ctx = lineal_app::RequestCtx {
        org_id: principal.org_id,
        actor: principal.actor,
        request_id: "req_me".into(),
        scopes: principal.scopes,
        team_scope: principal.team_scope.clone(),

        via_session: false,
        client_ip: None,
        credential_kind: None,
    };
    let (user, org) = match (
        app.get_user(&ctx, &user_id.to_string()).await,
        app.get_workspace(&ctx).await,
    ) {
        (Ok(u), Ok(o)) => (u, o),
        (Err(e), _) | (_, Err(e)) => return error_response(e),
    };
    let owned = app.owned_team_ids(&ctx).await.unwrap_or_default();
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
            "owner": owned.contains(&t.team.id),
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
    use lineal_testkit::{
        NeverMailer, RecordingMailer, SeedData, SeedUser, ctx_for, seed, test_app, test_app_with,
        test_app_with_mailer,
    };
    use tower::ServiceExt;

    /// Testpasswort für Alice: >= 12 Zeichen (Mindestlänge, LIN-62 L-8). Nicht das 10-stellige
    /// Seed-Passwort, weil `set_user_password` die Länge prüft.
    const PW: &str = "alice-test-pw-1";

    async fn setup(pool: sqlx::PgPool) -> Router {
        let store = PgStore::new(pool);
        let mut tx = store.begin(SeedData::acme().org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let app = test_app(Arc::new(store));
        // Seed-Hash ist Argon2; der Testkit-Hasher kennt nur `fake:`-Hashes -> Passwort setzen.
        app.set_user_password(&ctx_for(SeedUser::Alice), "me", PW)
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
        let (status, headers, body) = send(&r, login_req(PW)).await;
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
        // B5.36: die Session nennt, in welchen Teams der User Owner-Aktionen ausführen darf.
        let teams = body["teams"].as_array().unwrap();
        assert!(!teams.is_empty() && teams.iter().all(|t| t["owner"] == true));

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
        let (status, headers, body) = send(&r, login_req(PW)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert!(headers.contains_key(header::RETRY_AFTER));
        assert!(headers.get(header::SET_COOKIE).is_none());
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn invalid_session_cookie_is_cleared_both_names(pool: sqlx::PgPool) {
        // LIN-62 L-11: Eine Anfrage mit ungültigem Session-Cookie bekommt das Cookie per
        // `Max-Age=0` gelöscht – und zwar beide Namen (`__Host-` mit Secure und der einfache),
        // damit ein nach dem L-7-Rollout verbliebenes `lineal_session` sicher verschwindet.
        let r = setup(pool).await;
        let req = Request::builder()
            .uri("/auth/me")
            .header("cookie", "lineal_session=bogus-token")
            .body(Body::empty())
            .unwrap();
        let (status, headers, _) = send(&r, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let cookies: Vec<String> = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert!(
            cookies
                .iter()
                .any(|c| c.starts_with("__Host-lineal_session=")
                    && c.contains("Max-Age=0")
                    && c.contains("Secure")),
            "__Host-Cookie wird nicht gelöscht: {cookies:?}"
        );
        assert!(
            cookies
                .iter()
                .any(|c| c.starts_with("lineal_session=") && c.contains("Max-Age=0")),
            "einfaches Cookie wird nicht gelöscht: {cookies:?}"
        );
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn login_with_a_stale_cookie_still_sets_a_fresh_session(pool: sqlx::PgPool) {
        // Der Lösch-Guard darf das frische Login-Cookie nicht wieder entfernen, auch wenn die
        // Login-Anfrage ein altes, ungültiges Cookie mitschickt (LIN-62 L-11).
        let r = setup(pool).await;
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header("content-type", "application/json")
            .header("cookie", "lineal_session=stale")
            .body(Body::from(
                json!({ "email": "alice@acme.example", "password": PW }).to_string(),
            ))
            .unwrap();
        let (status, headers, _) = send(&r, req).await;
        assert_eq!(status, StatusCode::OK);
        let session: Vec<String> = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .filter(|c| c.starts_with("lineal_session=") || c.starts_with("__Host-lineal_session="))
            .collect();
        assert_eq!(
            session.len(),
            1,
            "genau ein Session-Cookie erwartet: {session:?}"
        );
        assert!(
            !session[0].contains("Max-Age=0"),
            "das Login-Cookie darf nicht gelöscht werden: {session:?}"
        );
    }

    fn accept_req(token: &str, email: &str, origin: Option<&str>) -> Request<Body> {
        let mut r = Request::builder()
            .method("POST")
            .uri("/invite/accept")
            .header("content-type", "application/json");
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        r.body(Body::from(
            json!({ "token": token, "email": email, "name": "Dora Neu", "password": "ein-gutes-passwort" })
                .to_string(),
        ))
        .unwrap()
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn invitation_can_be_accepted_without_a_session(pool: sqlx::PgPool) {
        let store = PgStore::new(pool);
        let mut tx = store.begin(SeedData::acme().org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let app = test_app(Arc::new(store));
        let created = app
            .invite_users(
                &ctx_for(SeedUser::Alice),
                lineal_app::usecases::invitation::InviteUsersInput {
                    emails: vec!["dora@acme.example".into()],
                    role: None,
                    teams: vec!["ENG".into()],
                },
            )
            .await
            .unwrap();
        let token = created[0].token.clone();
        let r = router(app, false);

        // Falscher Token und fremde E-Mail: einheitlich 404, kein Cookie.
        let (status, headers, _) =
            send(&r, accept_req("kein-token", "dora@acme.example", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(headers.get(header::SET_COOKIE).is_none());
        let (status, _, _) = send(&r, accept_req(&token, "mallory@evil.example", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Cross-Origin-Formular (CSRF): abgelehnt, bevor etwas passiert.
        let (status, _, _) = send(
            &r,
            accept_req(&token, "dora@acme.example", Some("https://evil.example")),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Richtig: Session-Cookie, und `/auth/me` zeigt den neuen User samt Team.
        let (status, headers, body) = send(
            &r,
            accept_req(&token, "dora@acme.example", Some("http://localhost:8080")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["workspace"], "acme");
        let cookie = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let me = Request::builder()
            .uri("/auth/me")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let (status, _, me) = send(&r, me).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["email"], "dora@acme.example");
        assert!(
            me["teams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["key"] == "ENG")
        );

        // Der Token ist verbraucht.
        let (status, _, _) = send(&r, accept_req(&token, "dora@acme.example", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
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

    // ----- LIN-58: Self-Service-Passwort-Reset ------------------------------------------------

    use lineal_app::usecases::auth::CreateApiKeyInput;
    use lineal_app::usecases::user::CreateUserInput;

    /// Extrahiert den Klartext-Token aus dem Reset-Link im Mail-Body.
    fn token_from_mail(body: &str) -> String {
        let start = body.find("token=").expect("token in mail") + "token=".len();
        body[start..]
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap()
            .to_string()
    }

    /// Seedt den Workspace, setzt Alices Passwort und liefert eine App mit aufzeichnendem Mailer.
    async fn reset_app(pool: sqlx::PgPool) -> (std::sync::Arc<lineal_app::App>, RecordingMailer) {
        let store = PgStore::new(pool);
        let mut tx = store.begin(SeedData::acme().org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let mailer = RecordingMailer::new();
        let app = test_app_with_mailer(
            std::sync::Arc::new(store),
            std::sync::Arc::new(mailer.clone()),
        );
        app.set_user_password(&ctx_for(SeedUser::Alice), "me", PW)
            .await
            .unwrap();
        (app, mailer)
    }

    const NEW_PW: &str = "fresh-password-abc-123";

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn reset_is_single_use_and_revokes_sessions_and_api_keys(pool: sqlx::PgPool) {
        let (app, mailer) = reset_app(pool).await;
        // Bestehende Zugangsdaten: eine Session und ein API-Key.
        let login = app
            .login(LoginInput {
                email: "alice@acme.example".into(),
                password: PW.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(app.principal_for_token(&login.token).await.is_ok());
        let key = app
            .create_api_key(
                &ctx_for(SeedUser::Alice),
                CreateApiKeyInput {
                    label: "ci".into(),
                    scopes: vec!["read".into()],
                    teams: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap()
            .key;
        assert!(app.principal_for_token(&key).await.is_ok());

        // Reset anfordern: genau eine Mail mit einem `lin_reset_`-Token.
        app.deliver_password_reset("alice@acme.example", None, None)
            .await
            .unwrap();
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1, "genau eine Reset-Mail");
        assert_eq!(sent[0].to, "alice@acme.example");
        let token = token_from_mail(&sent[0].body);
        assert!(token.starts_with("lin_reset_"), "Token: {token}");

        // Einlösen: Erfolg, danach sind Session UND API-Key ungültig.
        app.reset_password(&token, NEW_PW, None, None)
            .await
            .unwrap();
        assert!(
            app.principal_for_token(&login.token).await.is_err(),
            "alte Session muss widerrufen sein"
        );
        assert!(
            app.principal_for_token(&key).await.is_err(),
            "API-Key muss widerrufen sein"
        );
        // Einmalig: zweites Einlösen scheitert.
        assert!(
            app.reset_password(&token, "another-good-pw-999", None, None)
                .await
                .is_err(),
            "Token ist einmalig"
        );
        // Manipuliertes/unbekanntes Token scheitert.
        assert!(
            app.reset_password(&format!("{token}x"), NEW_PW, None, None)
                .await
                .is_err()
        );

        // Mit dem neuen Passwort kann sich Alice frisch anmelden.
        assert!(
            app.login(LoginInput {
                email: "alice@acme.example".into(),
                password: NEW_PW.into(),
                ..Default::default()
            })
            .await
            .is_ok()
        );

        // Audit-Eintrag `auth.password_reset` wurde geschrieben.
        let audit = app
            .list_audit_log(
                &ctx_for(SeedUser::Alice),
                lineal_app::usecases::audit::ListAuditInput {
                    action: Some("auth.password_reset".into()),
                    first: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(audit.entries.len(), 1, "ein Reset-Audit-Eintrag");
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn reset_mail_goes_to_the_stored_address_not_the_typed_one(pool: sqlx::PgPool) {
        let (app, mailer) = reset_app(pool).await;
        // Groß geschrieben eingegeben: die Mail geht trotzdem an die gespeicherte Adresse.
        app.deliver_password_reset("ALICE@ACME.EXAMPLE", None, None)
            .await
            .unwrap();
        assert_eq!(mailer.sent().len(), 1);
        assert_eq!(mailer.sent()[0].to, "alice@acme.example");
        // Nicht-ASCII-Eingabe wird gar nicht erst nachgeschlagen (kein `lower()`-Folding-Risiko).
        app.deliver_password_reset("alice@acme.examplе", None, None) // kyrillisches 'е'
            .await
            .unwrap();
        assert_eq!(mailer.sent().len(), 1, "Nicht-ASCII-Eingabe sendet nichts");
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn reset_requires_min_12_and_does_not_burn_the_token(pool: sqlx::PgPool) {
        let (app, mailer) = reset_app(pool).await;
        app.deliver_password_reset("alice@acme.example", None, None)
            .await
            .unwrap();
        let token = token_from_mail(&mailer.sent()[0].body);
        // Zu kurz: abgelehnt (Feld `password`), Token NICHT verbraucht.
        let err = app
            .reset_password(&token, "short", None, None)
            .await
            .unwrap_err();
        assert_eq!(err.details["field"], "password");
        // Gültiges Passwort löst dasselbe Token danach noch ein.
        app.reset_password(&token, NEW_PW, None, None)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn no_email_for_passwordless_bot_or_unknown_accounts(pool: sqlx::PgPool) {
        let (app, mailer) = reset_app(pool.clone()).await;
        // Passwortloses Konto (SSO-artig): per Einladung ohne Passwort angelegt.
        app.create_user(
            &ctx_for(SeedUser::Alice),
            CreateUserInput {
                name: "No Pass".into(),
                email: "nopass@acme.example".into(),
                role: lineal_domain::enums::UserRole::User,
                password: None,
                teams: vec!["ENG".into()],
            },
        )
        .await
        .unwrap();
        // Bot/`app`-Konto MIT Passwort-Hash (soll trotzdem keine Mail bekommen).
        {
            let store = PgStore::new(pool.clone());
            let mut tx = store.begin(SeedData::acme().org).await.unwrap();
            tx.users()
                .insert(&lineal_app::ports::repos::user::NewUser {
                    id: lineal_domain::ids::UserId::new_v7(),
                    name: "Bot".into(),
                    display_name: "botuser".into(),
                    email: "bot@acme.example".into(),
                    role: lineal_domain::enums::UserRole::App,
                    password_hash: Some("fake:whatever".into()),
                    now: app.clock.now(),
                })
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        for email in [
            "nopass@acme.example",
            "bot@acme.example",
            "ghost@acme.example",
        ] {
            app.deliver_password_reset(email, None, None).await.unwrap();
        }
        assert!(
            mailer.sent().is_empty(),
            "weder passwortlose noch Bot- noch unbekannte Konten bekommen eine Mail: {:?}",
            mailer.sent()
        );
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn expired_token_is_rejected(pool: sqlx::PgPool) {
        let (app, mailer) = reset_app(pool.clone()).await;
        app.deliver_password_reset("alice@acme.example", None, None)
            .await
            .unwrap();
        let token = token_from_mail(&mailer.sent()[0].body);
        // Zweite App auf DEMSELBEN Store, aber 31 Minuten später (TTL 30 min).
        let later = lineal_testkit::FixedClock(app.clock.now() + chrono::Duration::minutes(31));
        let app_later = test_app_with(
            std::sync::Arc::new(PgStore::new(pool.clone())),
            std::sync::Arc::new(later),
            std::sync::Arc::new(NeverMailer),
        );
        assert!(
            app_later
                .reset_password(&token, NEW_PW, None, None)
                .await
                .is_err(),
            "abgelaufenes Token muss abgelehnt werden"
        );
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn distributed_lockout_audit_is_recorded(pool: sqlx::PgPool) {
        let (app, _) = reset_app(pool).await;
        app.record_distributed_lockout("alice@acme.example", 42, 100, 900_000, false)
            .await
            .unwrap();
        let audit = app
            .list_audit_log(
                &ctx_for(SeedUser::Alice),
                lineal_app::usecases::audit::ListAuditInput {
                    action: Some("auth.distributed_lockout".into()),
                    first: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(audit.entries.len(), 1, "ein Lockout-Audit-Eintrag");
        assert_eq!(audit.entries[0].details["ipCount"], 42);
    }

    // ----- HTTP-Ebene --------------------------------------------------------------------------

    fn forgot_req(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/auth/forgot-password")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn forgot_password_response_is_identical_for_existing_unknown_and_rate_limited(
        pool: sqlx::PgPool,
    ) {
        let r = setup(pool).await;
        let existing = send(&r, forgot_req(json!({ "email": "alice@acme.example" }))).await;
        let unknown = send(&r, forgot_req(json!({ "email": "ghost@acme.example" }))).await;
        // Status und Body identisch; kein Cookie, kein Existenz-Signal.
        assert_eq!(existing.0, StatusCode::OK);
        assert_eq!((existing.0, &existing.2), (unknown.0, &unknown.2));
        assert!(existing.1.get(header::SET_COOKIE).is_none());

        // Über das Limit (3/Stunde je E-Mail): weiterhin dieselbe Antwort.
        let mut last = existing.clone();
        for _ in 0..5 {
            last = send(&r, forgot_req(json!({ "email": "alice@acme.example" }))).await;
        }
        assert_eq!(last.0, StatusCode::OK);
        assert_eq!(&last.2, &existing.2, "auch im Limit dieselbe Antwort");
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn forgot_password_returns_without_waiting_for_the_mailer(pool: sqlx::PgPool) {
        // Mailer, der nie zurückkehrt: der Handler darf nicht inline auf ihn warten.
        let store = PgStore::new(pool);
        let mut tx = store.begin(SeedData::acme().org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let app =
            test_app_with_mailer(std::sync::Arc::new(store), std::sync::Arc::new(NeverMailer));
        let r = router(app, false);
        let fut = send(&r, forgot_req(json!({ "email": "alice@acme.example" })));
        let (status, _, _) = tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .expect("Handler muss sofort antworten, nicht auf den Mailer warten");
        assert_eq!(status, StatusCode::OK);
    }

    #[sqlx::test(migrations = "../lineal-db/migrations")]
    async fn reset_password_endpoint_rejects_invalid_token_and_weak_password(pool: sqlx::PgPool) {
        let r = setup(pool).await;
        let reset = |body: serde_json::Value| {
            Request::builder()
                .method("POST")
                .uri("/auth/reset-password")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        // Unbekanntes Token: 400, neutraler Grund.
        let (status, _, body) = send(
            &r,
            reset(json!({ "token": "lin_reset_bogus", "newPassword": NEW_PW })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["details"]["reason"], "invalid_token");
        // Zu kurzes Passwort: 400, Feld `password` (vor dem Einlösen geprüft).
        let (status, _, body) = send(
            &r,
            reset(json!({ "token": "lin_reset_bogus", "newPassword": "short" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["details"]["field"], "password");
    }
}
