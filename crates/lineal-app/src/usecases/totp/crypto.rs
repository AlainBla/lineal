//! Envelope-Verschlüsselung für TOTP-Secrets.
//!
//! Pro Secret ein zufälliger Daten-Schlüssel (DEK), mit dem das Secret über
//! XChaCha20-Poly1305 verschlüsselt wird (24-Byte-Nonce vorangestellt). Der DEK
//! wird vom [`TotpKeyProvider`] mit dem KEK gewrappt. Gespeichert werden
//! `secret_enc`, `dek_wrapped`, Backend-Name und Schlüsselversion.

use crate::error::{AppError, AppResult};
use crate::ports::services::TotpKeyProvider;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rand::RngCore;

const DEK_LEN: usize = 32;
const XNONCE_LEN: usize = 24;

/// Ergebnis des Versiegelns — genau die zu persistierenden Spalten.
pub struct Sealed {
    pub secret_enc: Vec<u8>,
    pub dek_wrapped: Vec<u8>,
    pub backend: String,
    pub key_version: i32,
}

/// Verschlüsselt `plaintext` (das rohe TOTP-Secret) und wrappt den DEK über den Provider.
pub async fn seal(provider: &dyn TotpKeyProvider, plaintext: &[u8]) -> AppResult<Sealed> {
    let mut dek = [0u8; DEK_LEN];
    rand::rngs::OsRng.fill_bytes(&mut dek);
    let cipher = XChaCha20Poly1305::new_from_slice(&dek)
        .map_err(|_| AppError::internal("key setup failed"))?;

    let mut nonce_bytes = [0u8; XNONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| AppError::internal("encryption failed"))?;
    let mut secret_enc = nonce_bytes.to_vec();
    secret_enc.extend_from_slice(&ct);

    let dek_wrapped = provider.wrap_dek(&dek).await?;
    Ok(Sealed {
        secret_enc,
        dek_wrapped,
        backend: provider.backend().to_string(),
        key_version: provider.key_version(),
    })
}

/// Gegenstück zu [`seal`]: entpackt den DEK und entschlüsselt das Secret.
pub async fn open(
    provider: &dyn TotpKeyProvider,
    secret_enc: &[u8],
    dek_wrapped: &[u8],
) -> AppResult<Vec<u8>> {
    if secret_enc.len() <= XNONCE_LEN {
        return Err(AppError::internal("stored totp secret is corrupt"));
    }
    let dek = provider.unwrap_dek(dek_wrapped).await?;
    let cipher = XChaCha20Poly1305::new_from_slice(&dek)
        .map_err(|_| AppError::internal("key setup failed"))?;
    let (nonce, ct) = secret_enc.split_at(XNONCE_LEN);
    cipher
        .decrypt(XNonce::from_slice(nonce), ct)
        .map_err(|_| AppError::internal("stored totp secret cannot be decrypted"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Test-Provider: wrappt den DEK per XOR mit einem festen Schlüssel — reicht,
    /// um die Envelope-Mechanik (Trennung Secret/DEK) zu prüfen.
    struct XorProvider {
        key: [u8; 32],
    }
    #[async_trait]
    impl TotpKeyProvider for XorProvider {
        fn backend(&self) -> &'static str {
            "app"
        }
        fn key_version(&self) -> i32 {
            1
        }
        async fn wrap_dek(&self, dek: &[u8]) -> AppResult<Vec<u8>> {
            Ok(dek
                .iter()
                .zip(self.key.iter().cycle())
                .map(|(a, b)| a ^ b)
                .collect())
        }
        async fn unwrap_dek(&self, wrapped: &[u8]) -> AppResult<Vec<u8>> {
            Ok(wrapped
                .iter()
                .zip(self.key.iter().cycle())
                .map(|(a, b)| a ^ b)
                .collect())
        }
    }

    #[tokio::test]
    async fn seal_then_open_roundtrips() {
        let p = XorProvider { key: [7u8; 32] };
        let secret = b"12345678901234567890";
        let sealed = seal(&p, secret).await.unwrap();
        assert_eq!(sealed.backend, "app");
        assert_eq!(sealed.key_version, 1);
        // Secret taucht nicht im Klartext auf.
        assert!(!sealed.secret_enc.windows(secret.len()).any(|w| w == secret));
        let opened = open(&p, &sealed.secret_enc, &sealed.dek_wrapped)
            .await
            .unwrap();
        assert_eq!(opened, secret);
    }

    #[tokio::test]
    async fn open_with_wrong_key_fails() {
        let p = XorProvider { key: [7u8; 32] };
        let sealed = seal(&p, b"12345678901234567890").await.unwrap();
        let wrong = XorProvider { key: [9u8; 32] };
        assert!(
            open(&wrong, &sealed.secret_enc, &sealed.dek_wrapped)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn two_seals_differ() {
        let p = XorProvider { key: [7u8; 32] };
        let a = seal(&p, b"12345678901234567890").await.unwrap();
        let b = seal(&p, b"12345678901234567890").await.unwrap();
        assert_ne!(a.secret_enc, b.secret_enc, "zufälliger DEK/Nonce je Secret");
    }
}
