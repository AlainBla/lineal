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

impl Config {
    /// Liest die Konfiguration aus Umgebungsvariablen (siehe `.env.example`).
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Wie [`Config::from_env`], mit austauschbarer Quelle (Tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let session_secret =
            get("LINEAL_SESSION_SECRET").unwrap_or_else(|| DEFAULT_SESSION_SECRET.into());
        if session_secret.len() < 32 {
            bail!("LINEAL_SESSION_SECRET must be at least 32 bytes");
        }
        let base_url = get("LINEAL_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into());
        // Das Default-Secret ist öffentlich bekannt: hinter HTTPS (Produktion) nie erlaubt.
        if base_url.starts_with("https://") && session_secret == DEFAULT_SESSION_SECRET {
            bail!("LINEAL_SESSION_SECRET must be set when LINEAL_BASE_URL is https");
        }
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
            dev: get("LINEAL_DEV").is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes")),
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
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert_eq!(cfg.bind.port(), 8080);
        assert!(cfg.database_url.contains(":5440/"));
        assert!(!cfg.dev);
        assert!(
            Config::from_lookup(|k| (k == "LINEAL_SESSION_SECRET").then(|| "short".into()))
                .is_err()
        );
        assert!(Config::from_lookup(|k| (k == "LINEAL_BIND").then(|| "nope".into())).is_err());
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
        let cfg =
            Config::from_lookup(|k| (k == "LINEAL_BLOB_DIR").then(|| "/srv/blobs".into())).unwrap();
        assert_eq!(cfg.blob_dir, PathBuf::from("/srv/blobs"));
    }
}
