//! Standard-`TotpKeyProvider` (Backend `app`): der KEK wird per HKDF-SHA256 aus
//! dem Server-`session_secret` abgeleitet (Domain-Info `totp-kek-v1`), genau wie
//! `import.rs` seine At-Rest-Token ableitet. Kein zusätzliches Pflicht-Secret.
//! Ein expliziter 32-Byte-KEK kann per [`AppKeyProvider::from_raw_key`] gesetzt
//! werden (z. B. aus `LINEAL_TOTP_KEK`). Der DEK wird mit AES-256-GCM gewrappt,
//! Nonce vorangestellt.

use crate::error::{AppError, AppResult};
use crate::ports::services::TotpKeyProvider;
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use hkdf::Hkdf;
use sha2::Sha256;

const NONCE_LEN: usize = 12;
const KEK_INFO: &[u8] = b"totp-kek-v1";

pub struct AppKeyProvider {
    cipher: Aes256Gcm,
    key_version: i32,
}

impl AppKeyProvider {
    /// Leitet den KEK aus dem Session-Secret ab.
    pub fn from_session_secret(session_secret: &str) -> AppResult<Self> {
        let hk = Hkdf::<Sha256>::new(None, session_secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(KEK_INFO, &mut key)
            .map_err(|_| AppError::internal("totp key derivation failed"))?;
        Self::from_raw_key(&key)
    }

    /// Nutzt einen expliziten 32-Byte-KEK (z. B. aus `LINEAL_TOTP_KEK`).
    pub fn from_raw_key(key: &[u8]) -> AppResult<Self> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| AppError::internal("totp key setup failed"))?;
        Ok(Self {
            cipher,
            key_version: 1,
        })
    }
}

#[async_trait]
impl TotpKeyProvider for AppKeyProvider {
    fn backend(&self) -> &'static str {
        "app"
    }
    fn key_version(&self) -> i32 {
        self.key_version
    }
    async fn wrap_dek(&self, dek: &[u8]) -> AppResult<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, dek)
            .map_err(|_| AppError::internal("totp dek wrap failed"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }
    async fn unwrap_dek(&self, wrapped: &[u8]) -> AppResult<Vec<u8>> {
        if wrapped.len() <= NONCE_LEN {
            return Err(AppError::internal("wrapped totp dek is corrupt"));
        }
        let (nonce, ct) = wrapped.split_at(NONCE_LEN);
        self.cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| AppError::internal("wrapped totp dek cannot be unwrapped"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecases::totp::crypto::{open, seal};

    #[tokio::test]
    async fn app_key_envelope_roundtrips() {
        let p = AppKeyProvider::from_session_secret("a-session-secret-of-sufficient-length-xx")
            .unwrap();
        let secret = b"12345678901234567890";
        let sealed = seal(&p, secret).await.unwrap();
        let opened = open(&p, &sealed.secret_enc, &sealed.dek_wrapped)
            .await
            .unwrap();
        assert_eq!(opened, secret);
    }

    /// Known-Answer gegen die *alte* Generation: dieses Envelope wurde mit
    /// chacha20poly1305 0.10 (XChaCha) + hkdf 0.12 + aes-gcm 0.10 erzeugt. Die
    /// migrierten Libs (chacha 0.11 / hkdf 0.13) muessen es weiter entschluesseln,
    /// sonst werden bestehende gespeicherte TOTP-Secrets unlesbar. Bricht der
    /// Test, hat sich das At-Rest-Format geaendert -> Ursache suchen, nicht den
    /// Erwartungswert anpassen (Sicherheits-Invariante LIN-83).
    #[tokio::test]
    async fn opens_pre_migration_totp_envelope() {
        const SESSION: &str = "lin-83-kat-session-secret-0123456789";
        const SECRET_ENC_HEX: &str = "da1ecb78ac5893418758832d19251d59f86774137016d3eb4e080c544ad5b51ddc261faedd21c94fbce84453d03380aac1f26362ba30b02fcec025a7";
        const DEK_WRAPPED_HEX: &str = "55317150078d6c090673a1bb14dd2eb8d959b7aea1b7b1eaa6dd34eb2c517948e25682bd1c1095169795eb99778e1ba15bdad10804db0739a0e72abb";
        let p = AppKeyProvider::from_session_secret(SESSION).unwrap();
        let secret_enc = hex::decode(SECRET_ENC_HEX).unwrap();
        let dek_wrapped = hex::decode(DEK_WRAPPED_HEX).unwrap();
        let opened = open(&p, &secret_enc, &dek_wrapped).await.unwrap();
        assert_eq!(opened, b"12345678901234567890");
    }

    #[tokio::test]
    async fn different_session_secret_cannot_open() {
        let a =
            AppKeyProvider::from_session_secret("session-secret-aaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let b =
            AppKeyProvider::from_session_secret("session-secret-bbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let sealed = seal(&a, b"12345678901234567890").await.unwrap();
        assert!(
            open(&b, &sealed.secret_enc, &sealed.dek_wrapped)
                .await
                .is_err()
        );
    }
}
