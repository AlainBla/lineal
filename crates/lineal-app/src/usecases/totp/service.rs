//! LIN-70: Orchestrierende TOTP-Usecases auf `App` — Enrollment, Zweitschritt,
//! Recovery, Abschalten, Status, Reset. Die reine Mechanik liegt in den
//! Nachbar-Modulen (`core`, `crypto`, `recovery`); hier werden Transaktionen,
//! Policy (nur Owner darf fremd zurücksetzen) und Audit verdrahtet.

use super::{GRACE_MAX_DAYS, GRACE_MAX_LOGINS, core, crypto, recovery};
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::ports::repos::NewUserTotp;
use crate::resolve::user::actor_user_id;
use crate::usecases::audit::AuditEvent;
use crate::usecases::auth::Login;
use hkdf::Hkdf;
use lineal_domain::enums::UserRole;
use lineal_domain::ids::UserId;
use serde_json::json;
use sha2::Sha256;

const RECOVERY_KDF_INFO: &[u8] = b"totp-recovery-v1";

/// Ergebnis des Enrollment-Starts: was im QR/manuell angezeigt wird.
#[derive(Debug, Clone)]
pub struct TotpEnrollment {
    pub secret_base32: String,
    pub otpauth_uri: String,
}

/// Zweiter Faktor am Login: TOTP-Code oder Recovery-Code.
#[derive(Debug, Clone)]
pub enum TotpSecondFactor {
    Code(String),
    Recovery(String),
}

/// Statusanzeige fürs Account-Panel und das Enforcement-Banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpStatus {
    pub enabled: bool,
    pub confirmed: bool,
    pub recovery_remaining: i64,
    pub owner_enforced: bool,
    pub grace_days_left: Option<i64>,
    pub grace_logins_left: Option<i32>,
}

/// Was beim Bestätigen mit der Session geschehen soll.
#[derive(Debug, Clone)]
pub enum EnrollSession {
    /// Freiwilliges Enrollment aus den Einstellungen: die übrigen Sessions des
    /// Nutzers widerrufen, die aktuelle (sofern bekannt) behalten.
    RevokeOthers { keep_token_hash: Option<String> },
    /// Erzwungenes Enrollment (Owner ohne Session): eine frische Session prägen.
    MintFresh {
        ip_address: Option<String>,
        user_agent: Option<String>,
        request_id: Option<String>,
    },
}

/// Ergebnis des Bestätigens: Recovery-Codes und ggf. eine frisch geprägte Session.
#[derive(Debug, Clone)]
pub struct EnrollConfirmed {
    pub recovery_codes: Vec<String>,
    pub login: Option<Login>,
}

impl App {
    fn recovery_key(&self) -> AppResult<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(None, self.config.session_secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(RECOVERY_KDF_INFO, &mut key)
            .map_err(|_| AppError::internal("recovery key derivation failed"))?;
        Ok(key)
    }

    fn require_user(ctx: &RequestCtx) -> AppResult<UserId> {
        actor_user_id(ctx).ok_or_else(|| AppError::unauthenticated("login required"))
    }

    /// Startet das Enrollment: erzeugt ein Secret, versiegelt es und speichert es
    /// unbestätigt. Gibt Base32-Secret und otpauth-URI für QR/manuelle Eingabe zurück.
    pub async fn totp_begin_enrollment(&self, ctx: &RequestCtx) -> AppResult<TotpEnrollment> {
        let user_id = Self::require_user(ctx)?;
        let secret = core::generate_secret();
        let sealed = crypto::seal(self.totp_keys.as_ref(), &secret).await?;
        let now = self.clock.now();
        let mut tx = self.store.begin(ctx.org_id).await?;
        // Ein laufendes Enrollment darf ein bereits aktives TOTP nicht ueberschreiben
        // (sonst liesse sich 2FA durch ein abgebrochenes Enrollment deaktivieren).
        if tx.totp().get(user_id).await?.is_some_and(|t| t.is_active()) {
            return Err(AppError::invalid_request("two-factor is already active"));
        }
        let user = tx
            .users()
            .get(user_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        tx.totp()
            .upsert_pending(&NewUserTotp {
                user_id,
                org_id: ctx.org_id,
                secret_enc: sealed.secret_enc,
                dek_wrapped: sealed.dek_wrapped,
                key_backend: sealed.backend,
                key_version: sealed.key_version,
                now,
            })
            .await?;
        tx.commit().await?;
        Ok(TotpEnrollment {
            secret_base32: core::base32_encode(&secret),
            otpauth_uri: core::otpauth_uri("Lineal", &user.email, &secret),
        })
    }

    /// Bestätigt das Enrollment mit dem ersten Code, aktiviert TOTP und gibt den
    /// (einmalig sichtbaren) Satz Recovery-Codes zurück. Löscht eine laufende Schonfrist.
    pub async fn totp_confirm_enrollment(
        &self,
        ctx: &RequestCtx,
        code: &str,
        session: EnrollSession,
    ) -> AppResult<EnrollConfirmed> {
        let user_id = Self::require_user(ctx)?;
        let now = self.clock.now();
        let mut tx = self.store.begin(ctx.org_id).await?;
        let totp = tx
            .totp()
            .get(user_id)
            .await?
            .ok_or_else(|| AppError::invalid_request("no enrollment in progress"))?;
        if totp.is_active() {
            return Err(AppError::invalid_request("two-factor is already active"));
        }
        let secret =
            crypto::open(self.totp_keys.as_ref(), &totp.secret_enc, &totp.dek_wrapped).await?;
        // Nur prüfen, nicht als benutzt markieren: der Replay-Schutz (last_used_step)
        // greift am Login-Pfad, sonst scheiterte ein Login im selben 30-s-Fenster wie
        // die Bestätigung.
        core::verify(&secret, code, now.timestamp(), None)
            .ok_or_else(|| AppError::invalid_request("invalid code"))?;
        tx.totp().confirm(user_id, now).await?;
        let recovery_codes = self
            .store_fresh_recovery_codes(&mut tx, user_id, ctx.org_id, now)
            .await?;
        tx.totp().grace_clear(user_id).await?;

        let login = match session {
            EnrollSession::RevokeOthers { keep_token_hash } => {
                let keep = keep_token_hash.as_deref().unwrap_or("");
                tx.auth().revoke_other_sessions(user_id, keep, now).await?;
                None
            }
            EnrollSession::MintFresh {
                ip_address,
                user_agent,
                request_id,
            } => {
                let user = tx
                    .users()
                    .get(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("user"))?;
                Some(
                    self.mint_session_tx(
                        &mut *tx, user_id, ctx.org_id, user.role, ip_address, user_agent,
                        request_id, now,
                    )
                    .await?,
                )
            }
        };
        tx.commit().await?;
        Ok(EnrollConfirmed {
            recovery_codes,
            login,
        })
    }

    async fn store_fresh_recovery_codes(
        &self,
        tx: &mut Box<dyn crate::ports::Tx>,
        user_id: UserId,
        org_id: lineal_domain::ids::OrgId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<String>> {
        let codes = recovery::generate_codes();
        let key = self.recovery_key()?;
        let hashes: Vec<Vec<u8>> = codes
            .iter()
            .map(|c| recovery::hash_code(&key, c).to_vec())
            .collect();
        tx.totp()
            .replace_recovery_codes(user_id, org_id, &hashes, now)
            .await?;
        Ok(codes)
    }

    /// Zweiter Login-Schritt: prüft TOTP-Code (mit Replay-Schutz) oder Recovery-Code
    /// und prägt bei Erfolg die Session. Fehler überlässt der Aufrufer dem Lockout.
    #[allow(clippy::too_many_arguments)]
    pub async fn totp_complete_login(
        &self,
        user_id: UserId,
        org_id: lineal_domain::ids::OrgId,
        factor: &TotpSecondFactor,
        ip_address: Option<String>,
        user_agent: Option<String>,
        request_id: Option<String>,
    ) -> AppResult<Login> {
        let now = self.clock.now();
        let mut tx = self.store.begin(org_id).await?;
        let totp = tx
            .totp()
            .get(user_id)
            .await?
            .filter(|t| t.is_active())
            .ok_or_else(|| AppError::invalid_request("two-factor is not enabled"))?;
        let user = tx
            .users()
            .get(user_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        // Zwischen Passwortschritt und Zweitschritt kann der Nutzer gesperrt worden
        // sein (der Pending-Token lebt 5 min) — vor dem Praegen erneut pruefen.
        if !user.active {
            return Err(AppError::unauthenticated("account is not active"));
        }

        let ok = match factor {
            TotpSecondFactor::Code(code) => {
                let secret =
                    crypto::open(self.totp_keys.as_ref(), &totp.secret_enc, &totp.dek_wrapped)
                        .await?;
                match core::verify(&secret, code, now.timestamp(), totp.last_used_step) {
                    // set_last_used_step ist bedingt (nur vorwaerts): trifft es 0 Zeilen,
                    // hat parallel schon jemand denselben Code eingeloest -> kein Erfolg.
                    Some(step) => tx.totp().set_last_used_step(user_id, step).await?,
                    None => false,
                }
            }
            TotpSecondFactor::Recovery(input) => {
                let key = self.recovery_key()?;
                let rows = tx.totp().unused_recovery_codes(user_id).await?;
                let hit = rows
                    .iter()
                    .find(|r| recovery::verify_code(&key, input, &r.code_hash))
                    .map(|r| r.id);
                match hit {
                    // consume gibt false, wenn der Code parallel schon verbraucht wurde.
                    Some(id) => tx.totp().consume_recovery_code(id, now).await?,
                    None => false,
                }
            }
        };
        if !ok {
            return Err(AppError::unauthenticated("invalid second factor"));
        }
        let login = self
            .mint_session_tx(
                &mut *tx, user_id, org_id, user.role, ip_address, user_agent, request_id, now,
            )
            .await?;
        tx.commit().await?;
        Ok(login)
    }

    /// Schaltet TOTP ab. Für erzwungene Owner gesperrt, sonst würde die Pflicht umgangen.
    pub async fn totp_disable(&self, ctx: &RequestCtx) -> AppResult<()> {
        let user_id = Self::require_user(ctx)?;
        if self.config.totp_owner_enforcement
            && matches!(
                ctx.actor,
                Actor::User {
                    role: UserRole::Owner,
                    ..
                }
            )
        {
            return Err(AppError::forbidden("owners must keep two-factor enabled"));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.totp().delete_all(user_id, ctx.org_id).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new("auth.totp_disabled", "user", Some(user_id.0), json!({})),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Erzeugt einen frischen Satz Recovery-Codes und verwirft die alten.
    pub async fn totp_regenerate_recovery(&self, ctx: &RequestCtx) -> AppResult<Vec<String>> {
        let user_id = Self::require_user(ctx)?;
        let now = self.clock.now();
        let mut tx = self.store.begin(ctx.org_id).await?;
        let active = tx.totp().get(user_id).await?.is_some_and(|t| t.is_active());
        if !active {
            return Err(AppError::invalid_request("two-factor is not enabled"));
        }
        let codes = self
            .store_fresh_recovery_codes(&mut tx, user_id, ctx.org_id, now)
            .await?;
        tx.commit().await?;
        Ok(codes)
    }

    /// Statusanzeige fürs Account-Panel inkl. verbleibender Schonfrist.
    pub async fn totp_status(&self, ctx: &RequestCtx) -> AppResult<TotpStatus> {
        let user_id = Self::require_user(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let totp = tx.totp().get(user_id).await?;
        let confirmed = totp.as_ref().is_some_and(|t| t.is_active());
        let enabled = totp.is_some();
        let recovery_remaining = if confirmed {
            tx.totp().unused_recovery_count(user_id).await?
        } else {
            0
        };
        let is_owner = matches!(
            ctx.actor,
            Actor::User {
                role: UserRole::Owner,
                ..
            }
        );
        let owner_enforced = is_owner && self.config.totp_owner_enforcement;
        let (grace_days_left, grace_logins_left) = if owner_enforced && !confirmed {
            match tx.totp().grace_get(user_id).await? {
                Some(g) => {
                    let used_days = (self.clock.now() - g.started_at).num_days();
                    (
                        Some((GRACE_MAX_DAYS - used_days).max(0)),
                        Some((GRACE_MAX_LOGINS - g.logins_since).max(0)),
                    )
                }
                None => (Some(GRACE_MAX_DAYS), Some(GRACE_MAX_LOGINS)),
            }
        } else {
            (None, None)
        };
        Ok(TotpStatus {
            enabled,
            confirmed,
            recovery_remaining,
            owner_enforced,
            grace_days_left,
            grace_logins_left,
        })
    }

    /// Setzt das TOTP eines anderen Nutzers zurück (Geräteverlust). Nur Owner.
    pub async fn totp_reset_for_user(&self, ctx: &RequestCtx, target: UserId) -> AppResult<()> {
        if !matches!(
            ctx.actor,
            Actor::User {
                role: UserRole::Owner,
                ..
            }
        ) {
            return Err(AppError::forbidden("only owners can reset two-factor"));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        // Ziel muss zum Workspace gehoeren, sonst waere ein Cross-Tenant-Reset moeglich.
        tx.users()
            .get(target)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        tx.totp().delete_all(target, ctx.org_id).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new("auth.totp_reset", "user", Some(target.0), json!({})),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
