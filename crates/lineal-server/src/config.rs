//! Konfiguration aus der Umgebung (12-Factor). WP03 erweitert.

use anyhow::{Context, Result, bail};
use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_DATABASE_URL: &str = "postgres://lineal:lineal@localhost:5440/lineal";
const DEFAULT_SESSION_SECRET: &str = "change-me-32-bytes-minimum-xxxxxxxx";

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub base_url: String,
    pub session_secret: String,
    pub blob_dir: PathBuf,
    pub bind: SocketAddr,
    /// Entwicklungsmodus (GraphiQL, lockere Defaults).
    pub dev: bool,
    pub db_max_connections: u32,
}

/// 256 Bit Zufall (zwei UUIDv4) als Hex – nur für den Dev-Modus ohne gesetztes Secret.
fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl Config {
    /// Liest die Konfiguration aus Umgebungsvariablen (siehe `.env.example`).
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Wie [`Config::from_env`], mit austauschbarer Quelle (Tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let dev = get("LINEAL_DEV").is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"));
        // Das Secret signiert Blob-URLs (HMAC). Ein bekanntes oder fehlendes Secret erlaubt
        // gefälschte Upload-/Download-URLs, daher außerhalb des Dev-Modus Pflicht.
        let session_secret = match get("LINEAL_SESSION_SECRET") {
            Some(s) => s,
            None if dev => random_secret(),
            None => bail!("LINEAL_SESSION_SECRET must be set (or run with LINEAL_DEV=1)"),
        };
        if session_secret.len() < 32 {
            bail!("LINEAL_SESSION_SECRET must be at least 32 bytes");
        }
        if session_secret == DEFAULT_SESSION_SECRET && !dev {
            bail!("LINEAL_SESSION_SECRET still has the example value from .env.example");
        }
        let base_url = get("LINEAL_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into());
        Ok(Self {
            database_url: get("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.into()),
            base_url,
            session_secret,
            blob_dir: get("LINEAL_BLOB_DIR")
                .map_or_else(|| PathBuf::from("./var/blobs"), PathBuf::from),
            bind: get("LINEAL_BIND")
                .unwrap_or_else(|| "127.0.0.1:8080".into())
                .parse()
                .context("LINEAL_BIND must be host:port")?,
            dev,
            db_max_connections: get("LINEAL_DB_MAX_CONNECTIONS")
                .map(|v| v.parse())
                .transpose()
                .context("LINEAL_DB_MAX_CONNECTIONS must be a number")?
                .unwrap_or(10),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_validation() {
        let dev = |k: &str| (k == "LINEAL_DEV").then(|| "1".into());
        let cfg = Config::from_lookup(dev).unwrap();
        assert_eq!(cfg.bind.port(), 8080);
        assert!(cfg.database_url.contains(":5440/"));
        assert!(cfg.dev);
        assert!(cfg.session_secret.len() >= 32);
        assert_ne!(cfg.session_secret, DEFAULT_SESSION_SECRET);
        assert!(
            Config::from_lookup(|k| (k == "LINEAL_SESSION_SECRET").then(|| "short".into()))
                .is_err()
        );
        assert!(Config::from_lookup(|k| (k == "LINEAL_BIND").then(|| "nope".into())).is_err());
    }

    #[test]
    fn secret_is_required_outside_dev() {
        // Ohne Dev-Modus: kein Secret -> Fehler; Beispiel-Secret -> Fehler (auch über http).
        assert!(Config::from_lookup(|_| None).is_err());
        let example =
            |k: &str| (k == "LINEAL_SESSION_SECRET").then(|| DEFAULT_SESSION_SECRET.into());
        assert!(Config::from_lookup(example).is_err());
        let example_dev = |k: &str| match k {
            "LINEAL_SESSION_SECRET" => Some(DEFAULT_SESSION_SECRET.into()),
            "LINEAL_DEV" => Some("1".into()),
            _ => None,
        };
        assert!(Config::from_lookup(example_dev).is_ok());
    }

    #[test]
    fn default_secret_is_rejected_behind_https() {
        let https = |k: &str| (k == "LINEAL_BASE_URL").then(|| "https://lineal.example".into());
        assert!(Config::from_lookup(https).is_err());
        let with_secret = |k: &str| match k {
            "LINEAL_BASE_URL" => Some("https://lineal.example".into()),
            "LINEAL_SESSION_SECRET" => Some("a-real-secret-with-at-least-32-bytes!".into()),
            _ => None,
        };
        assert!(Config::from_lookup(with_secret).is_ok());
    }

    #[test]
    fn blob_dir_is_configurable() {
        let cfg = Config::from_lookup(|k| match k {
            "LINEAL_BLOB_DIR" => Some("/srv/blobs".into()),
            "LINEAL_DEV" => Some("1".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.blob_dir, PathBuf::from("/srv/blobs"));
    }
}
