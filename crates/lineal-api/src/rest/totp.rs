//! LIN-70: TOTP-REST-Endpunkte. Login-Zweitschritt (`/auth/login/totp`) und
//! Verwaltung (`/auth/totp/*`). Enrollment akzeptiert entweder eine Session
//! (freiwillig aus den Einstellungen) oder einen `enroll`-Pending-Token (ein
//! Owner, dessen Schonfrist abgelaufen ist und der noch keine Session hat).

use crate::auth_failures::{ACCOUNT_RULE, AuthFailures};
use crate::client_ip::ClientIp;
use crate::middleware::auth::blocked_response;
use crate::middleware::request_id::RequestId;
use crate::rest::auth::{error_response, session_cookie, session_cookie_value};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use lineal_app::usecases::auth::{SESSION_TTL_DAYS, hash_token};
use lineal_app::usecases::totp::pending::{PendingClaims, PendingScope, verify as verify_pending};
use lineal_app::usecases::totp::service::{EnrollSession, TotpSecondFactor};
use lineal_app::{Actor, App, AppError, RequestCtx};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::UserId;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Baut einen `RequestCtx` aus dem Session-Cookie. TOTP-Verwaltung ist bewusst
/// **nur per Browser-Session** erlaubt — ein API-Key/OAuth-Token (Bearer) darf 2FA
/// nicht einrichten, abschalten oder zurücksetzen.
async fn ctx_from_session(app: &App, headers: &HeaderMap) -> Result<RequestCtx, Response> {
    let token = session_cookie_value(headers)
        .ok_or_else(|| error_response(AppError::unauthenticated("session required")))?;
    let principal = app
        .principal_for_token(&token)
        .await
        .map_err(error_response)?;
    if principal.kind != lineal_app::ports::repos::CredentialKind::Session {
        return Err(error_response(AppError::unauthenticated(
            "session required",
        )));
    }
    Ok(RequestCtx {
        org_id: principal.org_id,
        actor: principal.actor,
        request_id: "req_totp".into(),
        scopes: principal.scopes,
        team_scope: principal.team_scope.clone(),
        via_session: true,
        client_ip: None,
        credential_kind: Some("session"),
    })
}

/// `RequestCtx` aus einem `enroll`-Pending-Token (Owner ohne Session).
#[allow(clippy::result_large_err)]
fn ctx_from_enroll_token(app: &App, token: &str) -> Result<(RequestCtx, PendingClaims), Response> {
    let claims = verify_pending(&app.config.session_secret, token, app.clock.now())
        .filter(|c| c.scope == PendingScope::Enroll)
        .ok_or_else(|| error_response(AppError::unauthenticated("invalid or expired token")))?;
    let ctx = RequestCtx {
        org_id: claims.org_id,
        // Enroll-Token werden nur fuer Owner ausgestellt.
        actor: Actor::User {
            id: claims.user_id,
            role: UserRole::Owner,
        },
        request_id: "req_totp_enroll".into(),
        scopes: lineal_app::context::Scopes::read_only(),
        team_scope: None,
        via_session: false,
        client_ip: None,
        credential_kind: None,
    };
    Ok((ctx, claims))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginTotpBody {
    pub pending_token: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub recovery_code: Option<String>,
}

/// `POST /auth/login/totp`: zweiter Login-Schritt. Prüft den Pending-Token (Scope
/// `login`), begrenzt Fehlversuche je Konto und prägt bei Erfolg die Session.
pub async fn login_totp(
    State(app): State<Arc<App>>,
    fails: Option<Extension<Arc<AuthFailures>>>,
    client: Option<Extension<ClientIp>>,
    request_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    Json(body): Json<LoginTotpBody>,
) -> Response {
    let now_ms = app.clock.now().timestamp_millis();
    let claims = match verify_pending(
        &app.config.session_secret,
        &body.pending_token,
        app.clock.now(),
    )
    .filter(|c| c.scope == PendingScope::Login)
    {
        Some(c) => c,
        None => return error_response(AppError::unauthenticated("invalid or expired token")),
    };

    // Konto-Lockout je Nutzer, atomar reserviert (wie der Login): ein Treffer bleibt
    // bei falschem Faktor stehen und zählt, bei Erfolg/Nicht-Auth-Fehler wird er
    // wieder freigegeben. Vermeidet das TOCTOU von check-dann-record.
    let lock_key = format!("totp:{}", claims.user_id.0);
    let reserved = match &fails {
        Some(Extension(f)) => match f.reserve(&lock_key, ACCOUNT_RULE, now_ms) {
            Ok(()) => true,
            Err(wait) => return blocked_response("/auth/login/totp", wait),
        },
        None => false,
    };
    let release = |reserved: bool| {
        if reserved {
            if let Some(Extension(f)) = &fails {
                f.release(&lock_key);
            }
        }
    };

    let factor = match (&body.code, &body.recovery_code) {
        (Some(code), _) if !code.trim().is_empty() => TotpSecondFactor::Code(code.clone()),
        (_, Some(rc)) if !rc.trim().is_empty() => TotpSecondFactor::Recovery(rc.clone()),
        _ => return error_response(AppError::invalid_request("code or recoveryCode required")),
    };

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(300).collect());
    let client_ip = client.map(|Extension(c)| c.0.to_string());
    let request_id = request_id.map(|Extension(r)| r.0);

    match app
        .totp_complete_login(
            claims.user_id,
            claims.org_id,
            &factor,
            client_ip,
            user_agent,
            request_id,
        )
        .await
    {
        Ok(login) => {
            release(reserved);
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
            // Ungültiger zweiter Faktor: Reservierung bleibt stehen (zählt). Andere
            // Fehler (z. B. Server) geben sie frei.
            if e.kind != lineal_app::ErrorKind::Unauthenticated {
                release(reserved);
            }
            error_response(e)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginBody {
    #[serde(default)]
    pub pending_token: Option<String>,
}

/// `POST /auth/totp/enroll/begin`: erzeugt ein Secret und gibt QR/otpauth zurück.
pub async fn enroll_begin(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<BeginBody>,
) -> Response {
    let ctx = match &body.pending_token {
        Some(token) => match ctx_from_enroll_token(&app, token) {
            Ok((ctx, _)) => ctx,
            Err(resp) => return resp,
        },
        None => match ctx_from_session(&app, &headers).await {
            Ok(ctx) => ctx,
            Err(resp) => return resp,
        },
    };
    match app.totp_begin_enrollment(&ctx).await {
        Ok(e) => (
            StatusCode::OK,
            Json(json!({ "secretBase32": e.secret_base32, "otpauthUri": e.otpauth_uri })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmBody {
    pub code: String,
    #[serde(default)]
    pub pending_token: Option<String>,
}

/// `POST /auth/totp/enroll/confirm`: aktiviert TOTP mit dem ersten Code und gibt
/// die Recovery-Codes zurück. Bei Session: andere Sessions werden widerrufen; bei
/// Enroll-Token: eine frische Session wird geprägt.
pub async fn enroll_confirm(
    State(app): State<Arc<App>>,
    client: Option<Extension<ClientIp>>,
    request_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    Json(body): Json<ConfirmBody>,
) -> Response {
    let (ctx, session) = match &body.pending_token {
        Some(token) => match ctx_from_enroll_token(&app, token) {
            Ok((ctx, _)) => (
                ctx,
                EnrollSession::MintFresh {
                    ip_address: client.map(|Extension(c)| c.0.to_string()),
                    user_agent: headers
                        .get(header::USER_AGENT)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.chars().take(300).collect()),
                    request_id: request_id.map(|Extension(r)| r.0),
                },
            ),
            Err(resp) => return resp,
        },
        None => match ctx_from_session(&app, &headers).await {
            Ok(ctx) => {
                let keep = session_cookie_value(&headers).map(|t| hash_token(&t));
                (
                    ctx,
                    EnrollSession::RevokeOthers {
                        keep_token_hash: keep,
                    },
                )
            }
            Err(resp) => return resp,
        },
    };

    let mint = matches!(session, EnrollSession::MintFresh { .. });
    match app.totp_confirm_enrollment(&ctx, &body.code, session).await {
        Ok(confirmed) => {
            let mut res = (
                StatusCode::OK,
                Json(json!({ "recoveryCodes": confirmed.recovery_codes })),
            )
                .into_response();
            if mint {
                if let Some(login) = confirmed.login {
                    if let Some(c) =
                        session_cookie(&app, &login.token, SESSION_TTL_DAYS * 24 * 3600)
                    {
                        res.headers_mut().append(header::SET_COOKIE, c);
                    }
                }
            }
            res
        }
        Err(e) => error_response(e),
    }
}

/// `POST /auth/totp/disable`: schaltet TOTP ab (für erzwungene Owner gesperrt).
pub async fn disable(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let ctx = match ctx_from_session(&app, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    match app.totp_disable(&ctx).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /auth/totp/recovery/regenerate`: erzeugt einen neuen Satz Recovery-Codes.
pub async fn regenerate_recovery(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let ctx = match ctx_from_session(&app, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    match app.totp_regenerate_recovery(&ctx).await {
        Ok(codes) => (StatusCode::OK, Json(json!({ "recoveryCodes": codes }))).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /auth/totp/status`: Status fürs Account-Panel und das Enforcement-Banner.
pub async fn status(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let ctx = match ctx_from_session(&app, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    match app.totp_status(&ctx).await {
        Ok(s) => (
            StatusCode::OK,
            Json(json!({
                "enabled": s.enabled,
                "confirmed": s.confirmed,
                "recoveryRemaining": s.recovery_remaining,
                "ownerEnforced": s.owner_enforced,
                "graceDaysLeft": s.grace_days_left,
                "graceLoginsLeft": s.grace_logins_left,
            })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetBody {
    pub user_id: String,
}

/// `POST /auth/totp/reset`: Owner setzt das TOTP eines anderen Nutzers zurück.
pub async fn reset(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<ResetBody>,
) -> Response {
    let ctx = match ctx_from_session(&app, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    let target = match uuid::Uuid::parse_str(&body.user_id) {
        Ok(u) => UserId(u),
        Err(_) => return error_response(AppError::invalid_request("invalid userId")),
    };
    match app.totp_reset_for_user(&ctx, target).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}
