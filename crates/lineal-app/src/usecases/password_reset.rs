//! usecases – Self-Service-Passwort-Reset (LIN-58).
//!
//! Zwei Schritte:
//!   1. `request` (`POST /auth/forgot-password`): Immer dieselbe, neutrale Antwort — ob ein Konto
//!      existiert oder nicht, verrät weder Status noch Body. Nur für existierende Konten mit
//!      Passwort-Login (keine Bots/`app`, keine passwortlosen/SSO-Konten) wird tatsächlich eine Mail
//!      verschickt. Die Zeitgleichheit stellt der REST-Handler her, indem er die gesamte Arbeit
//!      (Lookup + Schreiben + Versand) in einer losgelösten Task erledigt und sofort `200` liefert.
//!   2. `reset` (`POST /auth/reset-password`): Token (nur über seinen SHA-256-Hash nachgeschlagen)
//!      wird atomar und einmalig eingelöst; danach neues Passwort setzen, **alle** anderen
//!      Zugangsdaten widerrufen und `auth.password_reset` ins Audit-Log. KEINE Session — der Nutzer
//!      meldet sich frisch an.
//!
//! Token-Modell: `lin_reset_` + `random_hex()` (CSPRNG, > 256 Bit). Gespeichert wird nur
//! `hash_token` = SHA-256 (hex). Gültigkeit 30 min, Einmal-Nutzung.

use super::audit::AuditEvent;
use super::audit::user_ctx;
use super::auth::{hash_token, random_hex};
use super::user::{MAX_PASSWORD_LEN, validate_password};
use crate::app::App;
use crate::error::{AppError, AppResult};
use crate::ports::Mail;
use crate::ports::repos::auth::NewPasswordReset;
use chrono::Duration;
use lineal_domain::enums::UserRole;
use serde_json::json;

/// Präfix der Reset-Tokens (analog `lin_api_` für API-Keys).
pub const RESET_TOKEN_PREFIX: &str = "lin_reset_";
/// Gültigkeit eines Reset-Tokens in Minuten.
pub const RESET_TTL_MINUTES: i64 = 30;
/// Neustart-festes Konto-Limit je Stunde (Zeilen in `password_reset`). Die schnelle In-Memory-Sperre
/// (per E-Mail und per IP) im API-Adapter setzt dasselbe Limit vor jeder DB-Arbeit durch; dieser
/// Zähler übersteht zusätzlich einen Prozess-Neustart.
pub const RESET_MAX_PER_HOUR: i64 = 3;
/// Neustart-festes Konto-Limit je Tag.
pub const RESET_MAX_PER_DAY: i64 = 5;

/// Neutraler Fehler für den Reset-Schritt: unbekanntes, abgelaufenes oder bereits verbrauchtes Token
/// sind nicht unterscheidbar (kein Orakel). `reason` erlaubt der Oberfläche nur die Abgrenzung zum
/// Passwort-Validierungsfehler, verrät aber nichts über die Token-/Kontoexistenz.
fn invalid_reset_token() -> AppError {
    AppError::invalid_request("This password reset link is invalid or has expired.")
        .with_details(json!({ "reason": "invalid_token" }))
}

impl App {
    /// Reset-Mail mit dem Einmal-Link. Der Klartext-Token steht nur hier (und in der DB nur als Hash).
    fn password_reset_mail(&self, to: &str, token: &str) -> Mail {
        let link = format!(
            "{}/reset-password?token={token}",
            self.config.base_url.trim_end_matches('/')
        );
        Mail {
            to: to.to_string(),
            subject: lineal_domain::rules::text::sanitize_header_value(
                "Reset your Lineal password",
            ),
            body: format!(
                "We received a request to reset the password for your Lineal account.\n\n\
                 Reset your password: {link}\n\n\
                 The link is valid for {RESET_TTL_MINUTES} minutes and can be used once. \
                 If you did not request this, you can safely ignore this email — your password stays unchanged."
            ),
        }
    }

    /// Verarbeitet eine Reset-Anfrage (siehe Modul-Doku). Liefert **immer** `Ok(())`, sofern keine
    /// Infrastruktur (DB) ausfällt; „kein Konto", „ambige Adresse" und „über dem Konto-Limit" sind
    /// von außen nicht unterscheidbar. Der Aufrufer (REST-Handler) ruft dies losgelöst auf und
    /// antwortet unabhängig davon sofort neutral.
    ///
    /// `url_key` grenzt — wie beim Login — auf einen Workspace ein, wenn die Adresse in mehreren
    /// existiert. Ohne `url_key` und bei mehreren passwort-fähigen Konten wird bewusst NICHTS
    /// verschickt (die Anfrage bleibt mehrdeutig); das Formular trägt das optionale Workspace-Feld.
    pub async fn deliver_password_reset(
        &self,
        email: &str,
        url_key: Option<&str>,
        ip: Option<&str>,
    ) -> AppResult<()> {
        let email = email.trim();
        // Leeren `urlKey` wie „nicht angegeben" behandeln.
        let url_key = url_key.map(str::trim).filter(|k| !k.is_empty());
        // Dieselbe CSPRNG-+Hash-Arbeit wie der echte Pfad (vgl. `burn_verify`), für alle Pfade, die
        // nichts senden — Zeitangleichung.
        let dummy = || {
            let _ = hash_token(&format!("{RESET_TOKEN_PREFIX}{}", random_hex()));
        };
        // Ungültige Eingaben gar nicht erst nachschlagen: `lower()` in der DB könnte sonst manche
        // Nicht-ASCII-Zeichen auf ASCII falten und so eine fremde Adresse auf ein echtes Konto
        // abbilden (der Token ginge dann an das falsche Postfach).
        if !super::user::is_valid_email(email) {
            dummy();
            return Ok(());
        }
        let mut sys = self.store.begin_system().await?;
        let candidates = sys.auth().find_login_candidates(email, url_key).await?;
        drop(sys);
        // Nur existierende, aktive Konten mit Passwort-Login: keine Bots (`app`), keine
        // passwortlosen/SSO-Konten.
        let eligible: Vec<_> = candidates
            .into_iter()
            .filter(|c| c.active && c.password_hash.is_some() && c.role != UserRole::App)
            .collect();

        // Genau ein passendes Konto ⇒ echter Versand. 0 oder mehrdeutig ⇒ Dummy-Arbeit, kein Versand.
        let [chosen] = eligible.as_slice() else {
            dummy();
            return Ok(());
        };
        let chosen = chosen.clone();

        let now = self.clock.now();
        let token = format!("{RESET_TOKEN_PREFIX}{}", random_hex());
        let token_hash = hash_token(&token);

        let mut tx = self.store.begin(chosen.org_id).await?;
        // Gleichzeitige Anfragen je Konto serialisieren (wie das Einladungs-Limit LIN-64), damit das
        // Zähl-Limit nicht per Race umgangen wird und nicht zwei gültige Tokens entstehen.
        tx.advisory_lock("password_reset", Some(chosen.user_id.0))
            .await?;
        // Die Mail geht an die in der DB gespeicherte Adresse des Kontos — NIE an die (evtl. anders
        // normalisierte) Eingabe. Erneute Eignungsprüfung im Workspace (Wettlauf mit Sperre/Rolle).
        let Some(recipient) = tx
            .users()
            .get(chosen.user_id)
            .await?
            .filter(|u| u.active && u.role != UserRole::App)
        else {
            return Ok(());
        };
        // Neustart-festes Konto-Limit (analog Einladungs-Limit LIN-64). Über dem Limit: still nicht
        // senden (keine Enttarnung), die neutrale Antwort bleibt gleich.
        let last_hour = tx
            .auth()
            .count_password_resets_since(chosen.user_id, now - Duration::hours(1))
            .await?;
        let last_day = tx
            .auth()
            .count_password_resets_since(chosen.user_id, now - Duration::days(1))
            .await?;
        if last_hour >= RESET_MAX_PER_HOUR || last_day >= RESET_MAX_PER_DAY {
            return Ok(());
        }
        // Frühere, noch offene Tokens entwerten (used_at), dann das neue anlegen.
        tx.auth()
            .invalidate_password_resets(chosen.user_id, now)
            .await?;
        tx.auth()
            .insert_password_reset(&NewPasswordReset {
                id: self.ids.new_uuid(),
                org_id: chosen.org_id,
                user_id: chosen.user_id,
                token_hash,
                requested_ip: ip.map(str::to_owned),
                expires_at: now + Duration::minutes(RESET_TTL_MINUTES),
                now,
            })
            .await?;
        tx.commit().await?;

        // Versand erst nach Commit (ein Mailfehler lässt das Token gültig – erneute Anfrage sendet neu).
        if let Err(e) = self
            .mailer
            .send(self.password_reset_mail(&recipient.email, &token))
            .await
        {
            tracing::warn!(error = %e, "password reset email not sent");
        }
        Ok(())
    }

    /// Löst ein Reset-Token ein (siehe Modul-Doku). Setzt KEINE Session.
    pub async fn reset_password(
        &self,
        token: &str,
        new_password: &str,
        ip: Option<String>,
        request_id: Option<String>,
    ) -> AppResult<()> {
        // Überlange Eingabe gar nicht erst hashen; hängt nur vom Passwort ab (kein Orakel).
        if new_password.chars().count() > MAX_PASSWORD_LEN {
            return Err(AppError::invalid_request("password is too long")
                .with_details(json!({ "field": "password" })));
        }
        // Passwort VOR dem Einlösen prüfen, damit ein zu kurzes Passwort das Token nicht verbrennt.
        validate_password(new_password)?;
        if token.is_empty() || token.len() > 512 {
            return Err(invalid_reset_token());
        }
        let token_hash = hash_token(token);

        // Workspace des Tokens ermitteln (mandantenübergreifend), dann in dessen Transaktion atomar
        // einlösen — `revoke_all_credentials_of_user` ist org-gebunden.
        let mut sys = self.store.begin_system().await?;
        let org = sys.auth().find_password_reset_org(&token_hash).await?;
        drop(sys);
        let Some(org_id) = org else {
            return Err(invalid_reset_token());
        };

        let now = self.clock.now();
        let mut tx = self.store.begin(org_id).await?;
        let user_id = tx
            .auth()
            .consume_password_reset(&token_hash, now)
            .await?
            .ok_or_else(invalid_reset_token)?;
        // Eignung zum Einlösezeitpunkt erneut prüfen: gesperrte/`app`-Konten lösen nicht ein.
        let user = tx
            .users()
            .get(user_id)
            .await?
            .filter(|u| u.active && u.role != UserRole::App)
            .ok_or_else(invalid_reset_token)?;

        let hash = self.hasher.hash(new_password).await?;
        tx.users().set_password_hash(user_id, &hash, now).await?;
        // Design LIN-58: ALLE anderen Zugangsdaten widerrufen (Sessions, API-Keys, OAuth, Kalender),
        // damit ein Reset ein übernommenes Konto wirklich räumt. (Anders als `change_own_password`,
        // das API-Keys/Kalender nur auf Wunsch mitnimmt und die aktuelle Session behält.)
        tx.auth()
            .revoke_all_credentials_of_user(user_id, now)
            .await?;
        let ctx = user_ctx(
            org_id,
            user_id,
            user.role,
            ip,
            request_id.unwrap_or_default(),
            None,
        );
        self.record_audit(
            &mut *tx,
            &ctx,
            AuditEvent::new("auth.password_reset", "user", Some(user_id.0), json!({})),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
