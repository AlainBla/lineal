//! usecases – Persönlicher Account (WP29): Profil, Passwort ändern, aktive Sessions.
//!
//! Alles wirkt nur auf den **aufrufenden** User (keine User-Referenz in den Eingaben, also kein
//! Account-Wechsel zu fremden Usern). Passwort- und Session-Operationen verlangen eine
//! Web-Session (`via_session`): Ein API-Key oder OAuth-Token darf weder das Passwort ändern noch
//! Sessions abmelden (wie beim Anlegen von API-Keys).

use super::audit::AuditEvent;
use super::team::emit;
use super::team::validate_timezone;
use super::user::validate_password;
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::ports::repos::user::{ProfilePatch, User};
use chrono::{DateTime, Utc};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::UserId;
use serde_json::json;
use uuid::Uuid;

/// Eingabe für `update_viewer`; `None` lässt das Feld unverändert, `Some(None)` leert es.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateViewerInput {
    pub name: Option<String>,
    /// Handle für `@`-Erwähnungen (`^[a-z0-9._-]{1,40}$`).
    pub display_name: Option<String>,
    /// `assetUrl` eines Uploads (`fileUpload`) dieses Workspaces; nur Bilder.
    pub avatar_url: Option<Option<String>>,
    /// IANA-Zeitzone.
    pub timezone: Option<Option<String>>,
}

/// Eine aktive Web-Session des Users (ohne Token).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub id: Uuid,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    /// Die Session, mit der diese Anfrage gestellt wurde.
    pub current: bool,
}

/// Höchstlänge eines Passworts (Schutz vor überlangen Eingaben für den Hash).
pub const MAX_PASSWORD_LEN: usize = 1024;
/// Größte erlaubte Avatar-Datei.
pub const MAX_AVATAR_BYTES: i64 = 5 * 1024 * 1024;
const AVATAR_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

fn field_error(field: &str, message: &str) -> AppError {
    AppError::invalid_request(message.to_owned()).with_details(json!({ "field": field }))
}

/// Der aufrufende User; System und Apps haben keinen persönlichen Account.
fn viewer_id(ctx: &RequestCtx) -> AppResult<UserId> {
    match ctx.actor {
        Actor::User { id, .. } => Ok(id),
        _ => Err(AppError::forbidden("Only users have a personal account.")),
    }
}

fn require_write(ctx: &RequestCtx) -> AppResult<()> {
    if ctx.scopes.write {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "Token scope does not allow write operations.",
        ))
    }
}

/// Passwort und Sessions: nur mit der Web-Session und deren Kennung.
fn require_session<'a>(ctx: &RequestCtx, current_hash: Option<&'a str>) -> AppResult<&'a str> {
    require_write(ctx)?;
    match (ctx.via_session, current_hash) {
        (true, Some(h)) => Ok(h),
        _ => Err(AppError::forbidden(
            "This action needs a signed-in browser session.",
        )),
    }
}

impl App {
    /// Der eigene User.
    pub async fn get_viewer(&self, ctx: &RequestCtx) -> AppResult<User> {
        let id = viewer_id(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.users()
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced User."))
    }

    /// Ändert das eigene Profil (Name, Anzeigename, Avatar, Zeitzone).
    pub async fn update_viewer(
        &self,
        ctx: &RequestCtx,
        input: UpdateViewerInput,
    ) -> AppResult<User> {
        let id = viewer_id(ctx)?;
        require_write(ctx)?;
        let mut patch = ProfilePatch::default();
        if let Some(n) = &input.name {
            patch.name = Some(lineal_domain::rules::text::validate_single_line_name(
                "name", n, 255,
            )?);
        }
        if let Some(tz) = &input.timezone {
            if let Some(tz) = tz {
                validate_timezone(tz)?;
            }
            patch.timezone = Some(tz.clone());
        }
        if let Some(avatar) = &input.avatar_url {
            // Nur eigene, hochgeladene Bilder (kein Tracking über fremde URLs).
            patch.avatar_url = Some(match avatar {
                None => None,
                Some(url) => {
                    let assets = self.resolve_assets(ctx, std::slice::from_ref(url)).await?;
                    let asset = assets.first().ok_or_else(|| {
                        field_error("avatarUrl", "avatarUrl must be the assetUrl of an upload")
                    })?;
                    let image = asset
                        .content_type
                        .as_deref()
                        .is_some_and(|t| AVATAR_TYPES.contains(&t));
                    if !image {
                        return Err(field_error(
                            "avatarUrl",
                            "the avatar must be a PNG, JPEG, GIF or WebP image",
                        ));
                    }
                    if asset.size.is_some_and(|s| s > MAX_AVATAR_BYTES) {
                        return Err(field_error("avatarUrl", "the avatar must be at most 5 MB"));
                    }
                    Some(url.trim().to_owned())
                }
            });
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let current = tx
            .users()
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced User."))?;
        if let Some(d) = &input.display_name {
            let d = d.trim();
            let ok = !d.is_empty()
                && d.len() <= 40
                && d.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
                && !matches!(d, "me" | "none");
            if !ok {
                return Err(field_error(
                    "displayName",
                    "displayName must be 1-40 characters of a-z, 0-9, `.`, `_`, `-`",
                ));
            }
            if !d.eq_ignore_ascii_case(&current.display_name)
                && tx.users().display_name_taken(d).await?
            {
                return Err(AppError::conflict("This display name is already taken")
                    .with_details(json!({ "field": "displayName" })));
            }
            patch.display_name = Some(d.to_owned());
        }
        let changed: Vec<&str> = [
            ("name", patch.name.is_some()),
            ("displayName", patch.display_name.is_some()),
            ("avatarUrl", patch.avatar_url.is_some()),
            ("timezone", patch.timezone.is_some()),
        ]
        .into_iter()
        .filter_map(|(k, c)| c.then_some(k))
        .collect();
        if changed.is_empty() {
            return Ok(current);
        }
        tx.users()
            .update_profile(id, &patch, self.clock.now())
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "user.profile_updated",
                "user",
                Some(id.0),
                json!({ "fields": changed }),
            ),
        )
        .await?;
        emit(&mut *tx, ctx, DomainEvent::UserUpdated { user_id: id }).await?;
        let user = tx
            .users()
            .get(id)
            .await?
            .ok_or_else(|| AppError::internal("user vanished"))?;
        tx.commit().await?;
        Ok(user)
    }

    /// Ändert das eigene Passwort. Das alte Passwort wird geprüft (ein Fehlversuch ist im
    /// Audit-Log festgehalten; die Drosselung übernimmt der Aufrufer über die Fehlversuchs-Zähler
    /// der Authentifizierung). Danach sind alle **anderen** Sessions widerrufen; die aktuelle
    /// bleibt gültig. Liefert die Zahl der abgemeldeten Sessions.
    pub async fn change_own_password(
        &self,
        ctx: &RequestCtx,
        current_session_hash: Option<&str>,
        current_password: &str,
        new_password: &str,
    ) -> AppResult<u64> {
        let id = viewer_id(ctx)?;
        let keep = require_session(ctx, current_session_hash)?;
        if new_password.chars().count() > MAX_PASSWORD_LEN
            || current_password.chars().count() > MAX_PASSWORD_LEN
        {
            return Err(field_error("password", "password is too long"));
        }
        validate_password(new_password)
            .map_err(|e| e.with_details(json!({ "field": "newPassword" })))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let stored = tx.users().get_password_hash(id).await?;
        let verified = match &stored {
            Some(h) => self.hasher.verify(current_password, h).unwrap_or(false),
            None => {
                self.burn_verify(current_password);
                false
            }
        };
        if !verified {
            self.record_audit(
                &mut *tx,
                ctx,
                AuditEvent::new(
                    "user.password_change_failed",
                    "user",
                    Some(id.0),
                    json!({ "reason": if stored.is_some() { "invalid_password" } else { "no_password" } }),
                ),
            )
            .await?;
            tx.commit().await?;
            return Err(
                AppError::invalid_request("The current password is incorrect.")
                    .with_details(json!({ "field": "currentPassword" })),
            );
        }
        if current_password == new_password {
            return Err(field_error(
                "newPassword",
                "the new password must differ from the current one",
            ));
        }
        let now = self.clock.now();
        let hash = self.hasher.hash(new_password)?;
        tx.users().set_password_hash(id, &hash, now).await?;
        let revoked = tx.auth().revoke_other_sessions(id, keep, now).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "user.password_changed",
                "user",
                Some(id.0),
                json!({ "revokedSessions": revoked }),
            ),
        )
        .await?;
        emit(&mut *tx, ctx, DomainEvent::UserUpdated { user_id: id }).await?;
        tx.commit().await?;
        Ok(revoked)
    }

    /// Eigene aktive Sessions (neueste zuerst); die aktuelle ist markiert.
    pub async fn list_own_sessions(
        &self,
        ctx: &RequestCtx,
        current_session_hash: Option<&str>,
    ) -> AppResult<Vec<SessionInfo>> {
        let id = viewer_id(ctx)?;
        if !ctx.via_session {
            return Err(AppError::forbidden(
                "This action needs a signed-in browser session.",
            ));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let rows = tx.auth().list_sessions(id, self.clock.now()).await?;
        Ok(rows
            .into_iter()
            .map(|s| SessionInfo {
                current: current_session_hash == Some(s.token_hash.as_str()),
                id: s.id,
                user_agent: s.user_agent,
                ip_address: s.ip_address,
                created_at: s.created_at,
                last_used_at: s.last_used_at,
                expires_at: s.expires_at,
            })
            .collect())
    }

    /// Meldet eine eigene Session ab. Fremde oder unbekannte Sessions sind `not_found`; die
    /// aktuelle Session lässt sich hier nicht abmelden (dafür gibt es den Logout).
    pub async fn revoke_own_session(
        &self,
        ctx: &RequestCtx,
        current_session_hash: Option<&str>,
        session_id: Uuid,
    ) -> AppResult<()> {
        let id = viewer_id(ctx)?;
        let keep = require_session(ctx, current_session_hash)?;
        let now = self.clock.now();
        let mut tx = self.store.begin(ctx.org_id).await?;
        let sessions = tx.auth().list_sessions(id, now).await?;
        let target = sessions
            .iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| AppError::not_found("Could not find referenced Session."))?;
        if target.token_hash == keep {
            return Err(field_error(
                "id",
                "the current session cannot be revoked here; sign out instead",
            ));
        }
        if !tx.auth().revoke_session_by_id(id, session_id, now).await? {
            return Err(AppError::not_found("Could not find referenced Session."));
        }
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "user.session_revoked",
                "user",
                Some(id.0),
                json!({ "sessionId": session_id.to_string() }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Meldet alle eigenen Sessions außer der aktuellen ab; liefert deren Zahl.
    pub async fn revoke_other_sessions(
        &self,
        ctx: &RequestCtx,
        current_session_hash: Option<&str>,
    ) -> AppResult<u64> {
        let id = viewer_id(ctx)?;
        let keep = require_session(ctx, current_session_hash)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let revoked = tx
            .auth()
            .revoke_other_sessions(id, keep, self.clock.now())
            .await?;
        if revoked > 0 {
            self.record_audit(
                &mut *tx,
                ctx,
                AuditEvent::new(
                    "user.sessions_revoked",
                    "user",
                    Some(id.0),
                    json!({ "count": revoked }),
                ),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }
}
