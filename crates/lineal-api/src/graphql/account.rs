//! GraphQL: persönlicher Account (WP29) – `viewer`, `viewerUpdate`, `passwordChange`, `sessions`,
//! `sessionRevoke`, `sessionsRevokeOthers`.
//!
//! Alles wirkt nur auf den aufrufenden User. Passwort und Sessions gehen nur mit der Web-Session
//! (Cookie), nie mit API-Key oder OAuth-Token. Die Passwortänderung läuft durch die Fehlversuchs-
//! Zähler der Authentifizierung (je Account und je IP).

use super::org::gql_err;
use crate::auth_failures::{ACCOUNT_RULE, AuthFailures, IP_RULE};
use crate::client_ip::ClientIp;
use crate::context::{app, request_ctx};
use crate::middleware::auth::CurrentSession;
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::user::User;
use lineal_app::usecases::account::{SessionInfo, UpdateViewerInput};
use std::sync::Arc;
use uuid::Uuid;

/// Der angemeldete User mit seinem Profil.
#[derive(SimpleObject)]
#[graphql(name = "Viewer")]
pub struct ViewerGql {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub email: String,
    pub role: String,
    /// Anzeige-URL des Avatars (hochgeladene Bilder als kurzlebige signierte URL).
    pub avatar_url: Option<String>,
    /// IANA-Zeitzone; `null` = die Zeitzone des Browsers.
    pub timezone: Option<String>,
    pub created_at: String,
}

/// Eine aktive Web-Session des Users.
#[derive(SimpleObject)]
#[graphql(name = "UserSession")]
pub struct UserSessionGql {
    pub id: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    /// Die Session dieser Anfrage.
    pub current: bool,
}

impl From<SessionInfo> for UserSessionGql {
    fn from(s: SessionInfo) -> Self {
        Self {
            id: s.id.to_string(),
            user_agent: s.user_agent,
            ip_address: s.ip_address,
            created_at: s.created_at.to_rfc3339(),
            last_used_at: s.last_used_at.map(|d| d.to_rfc3339()),
            expires_at: s.expires_at.to_rfc3339(),
            current: s.current,
        }
    }
}

/// Fehlend = unverändert, `null` = leeren.
#[derive(InputObject)]
pub struct ViewerUpdateInput {
    pub name: Option<String>,
    /// Handle für `@`-Erwähnungen (`a-z`, `0-9`, `.`, `_`, `-`; höchstens 40 Zeichen).
    pub display_name: Option<String>,
    /// `assetUrl` eines Bild-Uploads (`fileUpload`).
    pub avatar_url: MaybeUndefined<String>,
    pub timezone: MaybeUndefined<String>,
}

#[derive(SimpleObject)]
pub struct ViewerPayload {
    pub success: bool,
    pub viewer: ViewerGql,
}

#[derive(SimpleObject)]
pub struct PasswordChangePayload {
    pub success: bool,
    /// Zahl der dabei abgemeldeten anderen Sessions.
    pub revoked_sessions: i32,
}

#[derive(SimpleObject)]
pub struct SessionRevokePayload {
    pub success: bool,
    /// Zahl der abgemeldeten Sessions.
    pub count: i32,
}

fn patch_value<T>(v: MaybeUndefined<T>) -> Option<Option<T>> {
    match v {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(v) => Some(Some(v)),
    }
}

fn session_hash<'a>(ctx: &Context<'a>) -> Option<&'a str> {
    ctx.data_opt::<CurrentSession>().map(|s| s.0.as_str())
}

async fn viewer_gql(ctx: &Context<'_>, user: User) -> async_graphql::Result<ViewerGql> {
    let avatar_url = match user.avatar_url.clone() {
        None => None,
        Some(url) => {
            // Hochgeladene Bilder: kurzlebige signierte URL; sonst (Altbestand) der Rohwert.
            let resolved = app(ctx)?
                .resolve_assets(request_ctx(ctx)?, std::slice::from_ref(&url))
                .await
                .map_err(gql_err)?;
            Some(resolved.into_iter().next().map_or(url, |a| a.download_url))
        }
    };
    Ok(ViewerGql {
        id: user.id.to_string(),
        name: user.name,
        display_name: user.display_name,
        email: user.email,
        role: user.role.as_str().to_owned(),
        avatar_url,
        timezone: user.timezone,
        created_at: user.created_at.to_rfc3339(),
    })
}

/// Reserviert Fehlversuche für die Passwortänderung (je Account und je Client-IP) vor der
/// Prüfung. Zurückgegeben wird die Liste der Schlüssel; bei Erfolg gibt der Aufrufer sie frei.
fn reserve_password_attempt(
    ctx: &Context<'_>,
    user_id: &str,
    now_ms: i64,
) -> async_graphql::Result<Option<(Arc<AuthFailures>, Vec<String>)>> {
    let Some(fails) = ctx.data_opt::<Arc<AuthFailures>>() else {
        return Ok(None);
    };
    let mut held: Vec<String> = Vec::new();
    let mut keys = vec![(format!("acctpw:{user_id}"), ACCOUNT_RULE)];
    if let Some(ip) = ctx.data_opt::<ClientIp>() {
        keys.push((
            format!("ip:{}", crate::rate_limit::ip_bucket(ip.0)),
            IP_RULE,
        ));
    }
    for (key, rule) in keys {
        match fails.reserve(&key, rule, now_ms) {
            Ok(()) => held.push(key),
            Err(wait) => {
                for k in &held {
                    fails.release(k);
                }
                return Err(gql_err(
                    AppError::rate_limited("Too many failed attempts. Try again later.")
                        .with_details(serde_json::json!({ "retryAfter": wait })),
                ));
            }
        }
    }
    Ok(Some((fails.clone(), held)))
}

#[derive(Default)]
pub struct AccountQuery;

#[Object]
impl AccountQuery {
    /// Der angemeldete User.
    async fn viewer(&self, ctx: &Context<'_>) -> async_graphql::Result<ViewerGql> {
        let user = app(ctx)?
            .get_viewer(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        viewer_gql(ctx, user).await
    }

    /// Eigene aktive Web-Sessions (nur mit Browser-Session), neueste zuerst.
    async fn sessions(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<UserSessionGql>> {
        Ok(app(ctx)?
            .list_own_sessions(request_ctx(ctx)?, session_hash(ctx))
            .await
            .map_err(gql_err)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}

#[derive(Default)]
pub struct AccountMutation;

#[Object]
impl AccountMutation {
    /// Ändert das eigene Profil.
    async fn viewer_update(
        &self,
        ctx: &Context<'_>,
        input: ViewerUpdateInput,
    ) -> async_graphql::Result<ViewerPayload> {
        let user = app(ctx)?
            .update_viewer(
                request_ctx(ctx)?,
                UpdateViewerInput {
                    name: input.name,
                    display_name: input.display_name,
                    avatar_url: patch_value(input.avatar_url),
                    timezone: patch_value(input.timezone),
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(ViewerPayload {
            success: true,
            viewer: viewer_gql(ctx, user).await?,
        })
    }

    /// Ändert das eigene Passwort (altes Passwort nötig) und meldet alle anderen Sessions ab.
    /// Nur mit Browser-Session. Falsche alte Passwörter zählen als Fehlversuche (Sperre mit
    /// `rate_limited`).
    async fn password_change(
        &self,
        ctx: &Context<'_>,
        current_password: String,
        new_password: String,
    ) -> async_graphql::Result<PasswordChangePayload> {
        let rc = request_ctx(ctx)?;
        let app = app(ctx)?;
        let user_id = match rc.actor {
            lineal_app::Actor::User { id, .. } => id.to_string(),
            _ => {
                return Err(gql_err(AppError::forbidden(
                    "Only users can change a password.",
                )));
            }
        };
        let now_ms = app.clock.now().timestamp_millis();
        let reservation = reserve_password_attempt(ctx, &user_id, now_ms)?;
        let result = app
            .change_own_password(rc, session_hash(ctx), &current_password, &new_password)
            .await;
        match result {
            Ok(revoked) => {
                if let Some((fails, keys)) = reservation {
                    for k in &keys {
                        fails.release(k);
                    }
                }
                Ok(PasswordChangePayload {
                    success: true,
                    revoked_sessions: i32::try_from(revoked).unwrap_or(i32::MAX),
                })
            }
            Err(e) => {
                // Nur ein falsches altes Passwort zählt; Eingabefehler und Rechte nicht.
                let counts = e
                    .details
                    .get("field")
                    .is_some_and(|f| f == "currentPassword");
                if !counts && let Some((fails, keys)) = reservation {
                    for k in &keys {
                        fails.release(k);
                    }
                }
                Err(gql_err(e))
            }
        }
    }

    /// Meldet eine eigene Session ab (fremde IDs: `not_found`).
    async fn session_revoke(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<SessionRevokePayload> {
        let uuid = Uuid::parse_str(id.trim())
            .map_err(|_| gql_err(AppError::not_found("Could not find referenced Session.")))?;
        app(ctx)?
            .revoke_own_session(request_ctx(ctx)?, session_hash(ctx), uuid)
            .await
            .map_err(gql_err)?;
        Ok(SessionRevokePayload {
            success: true,
            count: 1,
        })
    }

    /// Meldet alle eigenen Sessions außer der aktuellen ab.
    async fn sessions_revoke_others(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<SessionRevokePayload> {
        let n = app(ctx)?
            .revoke_other_sessions(request_ctx(ctx)?, session_hash(ctx))
            .await
            .map_err(gql_err)?;
        Ok(SessionRevokePayload {
            success: true,
            count: i32::try_from(n).unwrap_or(i32::MAX),
        })
    }
}
