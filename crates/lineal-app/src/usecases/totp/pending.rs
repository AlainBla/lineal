//! Stateless Pending-Token für den zweiten Login-Schritt (LIN-70).
//!
//! Nach erfolgreichem Passwort, aber vor der Session, gibt der Server dem Client
//! einen kurzlebigen Token mit. Er trägt `user_id`, `org_id`, einen **Scope**
//! (`login` = TOTP-Code fällig · `enroll` = Owner muss jetzt einrichten) und ein
//! Ablaufdatum, signiert per HMAC-SHA256 mit einem aus dem `session_secret`
//! abgeleiteten Schlüssel. Zustandslos: der Token allein gewährt nichts — der
//! zweite Faktor (Code bzw. Enrollment) ist weiterhin nötig.

use crate::error::{AppError, AppResult};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use lineal_domain::ids::{OrgId, UserId};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Gültigkeit des Pending-Tokens.
pub const TTL_SECS: i64 = 300;
const KDF_INFO: &[u8] = b"totp-pending-v1";

/// Wofür der Token gilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingScope {
    /// Nutzer mit aktivem TOTP: Code fällig.
    Login,
    /// Owner ohne TOTP, Schonfrist abgelaufen: Enrollment erzwingen.
    Enroll,
}

impl PendingScope {
    fn as_str(self) -> &'static str {
        match self {
            PendingScope::Login => "login",
            PendingScope::Enroll => "enroll",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "login" => Some(PendingScope::Login),
            "enroll" => Some(PendingScope::Enroll),
            _ => None,
        }
    }
}

/// Entschlüsselte Claims eines gültigen Tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClaims {
    pub user_id: UserId,
    pub org_id: OrgId,
    pub scope: PendingScope,
}

fn mac_key(session_secret: &str) -> AppResult<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, session_secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(KDF_INFO, &mut key)
        .map_err(|_| AppError::internal("pending token key derivation failed"))?;
    Ok(key)
}

fn sign(key: &[u8], payload: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC akzeptiert jede Schlüssellänge");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Stellt einen Token aus.
pub fn mint(
    session_secret: &str,
    user_id: UserId,
    org_id: OrgId,
    scope: PendingScope,
    now: DateTime<Utc>,
) -> AppResult<String> {
    let exp = (now + Duration::seconds(TTL_SECS)).timestamp();
    let payload = format!("{}|{}|{}|{}", user_id.0, org_id.0, scope.as_str(), exp);
    let sig = sign(&mac_key(session_secret)?, &payload);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload.as_bytes()),
        URL_SAFE_NO_PAD.encode(sig)
    ))
}

/// Prüft Signatur und Ablauf und gibt die Claims zurück. `None` bei allem Ungültigen.
pub fn verify(session_secret: &str, token: &str, now: DateTime<Utc>) -> Option<PendingClaims> {
    let (payload_b64, sig_b64) = token.split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
    let key = mac_key(session_secret).ok()?;
    let expected = sign(&key, std::str::from_utf8(&payload).ok()?);
    if !bool::from(expected.ct_eq(&sig)) {
        return None;
    }
    let payload = std::str::from_utf8(&payload).ok()?;
    let mut parts = payload.split('|');
    let user_id = UserId(Uuid::parse_str(parts.next()?).ok()?);
    let org_id = OrgId(Uuid::parse_str(parts.next()?).ok()?);
    let scope = PendingScope::parse(parts.next()?)?;
    let exp: i64 = parts.next()?.parse().ok()?;
    if now.timestamp() >= exp {
        return None;
    }
    Some(PendingClaims {
        user_id,
        org_id,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (UserId, OrgId) {
        (UserId::new_v7(), OrgId::new_v7())
    }

    #[test]
    fn mint_then_verify_roundtrips() {
        let (u, o) = ids();
        let now = Utc::now();
        let t = mint(
            "secret-secret-secret-secret-xxxx",
            u,
            o,
            PendingScope::Login,
            now,
        )
        .unwrap();
        let c = verify("secret-secret-secret-secret-xxxx", &t, now).unwrap();
        assert_eq!(
            c,
            PendingClaims {
                user_id: u,
                org_id: o,
                scope: PendingScope::Login
            }
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let (u, o) = ids();
        let now = Utc::now();
        let t = mint(
            "secret-secret-secret-secret-xxxx",
            u,
            o,
            PendingScope::Enroll,
            now,
        )
        .unwrap();
        let later = now + Duration::seconds(TTL_SECS + 1);
        assert!(verify("secret-secret-secret-secret-xxxx", &t, later).is_none());
    }

    #[test]
    fn tampered_or_wrong_secret_is_rejected() {
        let (u, o) = ids();
        let now = Utc::now();
        let t = mint(
            "secret-secret-secret-secret-xxxx",
            u,
            o,
            PendingScope::Login,
            now,
        )
        .unwrap();
        // Falsches Secret.
        assert!(verify("another-secret-another-secret-yy", &t, now).is_none());
        // Manipulierte Payload.
        let mut bad = t.clone();
        bad.insert(0, 'A');
        assert!(verify("secret-secret-secret-secret-xxxx", &bad, now).is_none());
    }

    #[test]
    fn scope_is_carried() {
        let (u, o) = ids();
        let now = Utc::now();
        let t = mint(
            "secret-secret-secret-secret-xxxx",
            u,
            o,
            PendingScope::Enroll,
            now,
        )
        .unwrap();
        assert_eq!(
            verify("secret-secret-secret-secret-xxxx", &t, now)
                .unwrap()
                .scope,
            PendingScope::Enroll
        );
    }
}
