//! LIN-70: OpenBAO-/Vault-Transit-Backend fuer die TOTP-Secret-Verschluesselung.
//!
//! Der DEK wird nicht lokal, sondern von OpenBAO Transit ge-wrapped/ent-wrapped;
//! der KEK verlaesst OpenBAO nie. Der HTTP-Zugriff liegt hinter `BaoTransport`,
//! damit das Request-/Response-Format ohne echte Instanz testbar ist. Gegen eine
//! laufende Instanz wird manuell verifiziert (nicht in CI).

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use lineal_app::ports::TotpKeyProvider;
use lineal_app::{AppError, AppResult};
use serde_json::{Value, json};
use std::sync::Arc;

/// Minimaler Transport fuer OpenBAO-POSTs (injizierbar fuer Tests).
#[async_trait]
pub trait BaoTransport: Send + Sync {
    async fn post(&self, path: &str, body: Value) -> AppResult<Value>;
}

/// Echter HTTP-Transport (reqwest, rustls).
pub struct HttpTransport {
    addr: String,
    token: String,
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(addr: String, token: String) -> Self {
        Self {
            addr,
            token,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl BaoTransport for HttpTransport {
    async fn post(&self, path: &str, body: Value) -> AppResult<Value> {
        let resp = self
            .client
            .post(format!("{}{}", self.addr, path))
            .header("X-Vault-Token", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("openbao request failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::internal(format!(
                "openbao returned status {}",
                resp.status()
            )));
        }
        resp.json::<Value>()
            .await
            .map_err(|e| AppError::internal(format!("openbao response invalid: {e}")))
    }
}

/// `TotpKeyProvider` ueber OpenBAO Transit.
pub struct OpenBaoProvider {
    transport: Arc<dyn BaoTransport>,
    transit_key: String,
}

impl OpenBaoProvider {
    pub fn new(transport: Arc<dyn BaoTransport>, transit_key: String) -> Self {
        Self {
            transport,
            transit_key,
        }
    }
}

#[async_trait]
impl TotpKeyProvider for OpenBaoProvider {
    fn backend(&self) -> &'static str {
        "openbao"
    }
    fn key_version(&self) -> i32 {
        1
    }
    async fn wrap_dek(&self, dek: &[u8]) -> AppResult<Vec<u8>> {
        let resp = self
            .transport
            .post(
                &format!("/v1/transit/encrypt/{}", self.transit_key),
                json!({ "plaintext": B64.encode(dek) }),
            )
            .await?;
        let ciphertext = resp["data"]["ciphertext"]
            .as_str()
            .ok_or_else(|| AppError::internal("openbao encrypt: missing ciphertext"))?;
        Ok(ciphertext.as_bytes().to_vec())
    }
    async fn unwrap_dek(&self, wrapped: &[u8]) -> AppResult<Vec<u8>> {
        let ciphertext = std::str::from_utf8(wrapped)
            .map_err(|_| AppError::internal("wrapped dek is not utf8"))?;
        let resp = self
            .transport
            .post(
                &format!("/v1/transit/decrypt/{}", self.transit_key),
                json!({ "ciphertext": ciphertext }),
            )
            .await?;
        let plaintext_b64 = resp["data"]["plaintext"]
            .as_str()
            .ok_or_else(|| AppError::internal("openbao decrypt: missing plaintext"))?;
        B64.decode(plaintext_b64)
            .map_err(|_| AppError::internal("openbao plaintext is not base64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::usecases::totp::crypto::{open, seal};
    use std::sync::Mutex;

    /// Simuliert OpenBAO Transit: encrypt haengt `vault:v1:` vor den (base64-)
    /// Klartext, decrypt entfernt es wieder. Protokolliert die Pfade.
    struct FakeBao {
        paths: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BaoTransport for FakeBao {
        async fn post(&self, path: &str, body: Value) -> AppResult<Value> {
            self.paths.lock().unwrap().push(path.to_string());
            if path.contains("/encrypt/") {
                let pt = body["plaintext"].as_str().unwrap();
                Ok(json!({ "data": { "ciphertext": format!("vault:v1:{pt}") } }))
            } else {
                let ct = body["ciphertext"].as_str().unwrap();
                let pt = ct.strip_prefix("vault:v1:").unwrap();
                Ok(json!({ "data": { "plaintext": pt } }))
            }
        }
    }

    #[tokio::test]
    async fn wrap_then_unwrap_roundtrips_via_transit() {
        let fake = Arc::new(FakeBao {
            paths: Mutex::new(Vec::new()),
        });
        let provider = OpenBaoProvider::new(fake.clone(), "lineal-totp".into());
        assert_eq!(provider.backend(), "openbao");

        let dek = [7u8; 32];
        let wrapped = provider.wrap_dek(&dek).await.unwrap();
        // Der gewrappte Wert ist der Transit-Ciphertext, nicht der DEK.
        assert!(wrapped.starts_with(b"vault:v1:"));
        let unwrapped = provider.unwrap_dek(&wrapped).await.unwrap();
        assert_eq!(unwrapped, dek);

        let paths = fake.paths.lock().unwrap();
        assert_eq!(paths[0], "/v1/transit/encrypt/lineal-totp");
        assert_eq!(paths[1], "/v1/transit/decrypt/lineal-totp");
    }

    #[tokio::test]
    async fn full_envelope_through_openbao_provider() {
        let fake = Arc::new(FakeBao {
            paths: Mutex::new(Vec::new()),
        });
        let provider = OpenBaoProvider::new(fake, "k".into());
        let sealed = seal(&provider, b"12345678901234567890").await.unwrap();
        assert_eq!(sealed.backend, "openbao");
        let opened = open(&provider, &sealed.secret_enc, &sealed.dek_wrapped)
            .await
            .unwrap();
        assert_eq!(opened, b"12345678901234567890");
    }
}
