//! Konkrete Adapter für die Ports: System-Clock, UUID-v7-IdGen, lokaler BlobStore,
//! Argon2-Hasher, leere Suche, Log- oder SMTP-Mailer.

use crate::config::{Config, SmtpConfig};
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::{
    Clock, IdGen, Mail, Mailer, PasswordHasher, SearchHit, SearchIndex, Store,
};
use lineal_app::{App, AppConfig, AppError, AppResult};
use lineal_domain::ids::OrgId;
use std::sync::Arc;
use uuid::Uuid;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct UuidV7IdGen;

impl IdGen for UuidV7IdGen {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// Lokaler Blob-Store (Verzeichnis `LINEAL_BLOB_DIR`, HMAC-signierte URLs). Die Endpunkte
/// `/uploads/{token}` und `/files/{token}` liegen in `lineal-api::blob`.
pub type LocalBlobStore = lineal_api::blob::LocalBlobStore;

/// Argon2id-Passwort-Hashing (PHC-String mit zufälligem Salt, Standardparameter der Crate).
#[derive(Default)]
pub struct Argon2Hasher {
    argon2: Argon2<'static>,
}

impl PasswordHasher for Argon2Hasher {
    fn hash(&self, password: &str) -> AppResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        self.argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::internal(format!("password hashing failed: {e}")))
    }

    /// Falsches Passwort oder unlesbarer Hash ergeben `Ok(false)` (kein Hinweis für Angreifer).
    fn verify(&self, password: &str, hash: &str) -> AppResult<bool> {
        let Ok(parsed) = PasswordHash::new(hash) else {
            return Ok(false);
        };
        Ok(self
            .argon2
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }
}

/// Leere Suche bis WP11.
pub struct NoSearch;

#[async_trait]
impl SearchIndex for NoSearch {
    async fn search(&self, _org: OrgId, _query: &str, _limit: u32) -> AppResult<Vec<SearchHit>> {
        Ok(vec![])
    }
}

/// Schreibt Mails nur ins Log (lokale Entwicklung).
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: Mail) -> AppResult<()> {
        // Defensiv: Zeilenumbrüche/Steuerzeichen in Header-Feldern entfernen (Log-Forging, Header-Injection).
        let to = lineal_domain::rules::text::sanitize_header_value(&mail.to);
        let subject = lineal_domain::rules::text::sanitize_header_value(&mail.subject);
        tracing::info!(%to, %subject, "mail (not sent)");
        Ok(())
    }
}

/// Übergibt Mails per SMTP an einen Relay ohne Anmeldung (Postfix auf dem Host); Zustellung,
/// Wiederholung und DKIM übernimmt der Relay.
pub struct SmtpMailer {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: lettre::message::Mailbox,
}

impl SmtpMailer {
    pub fn new(cfg: &SmtpConfig) -> AppResult<Self> {
        let from = cfg.from.parse().map_err(|e| {
            AppError::internal(format!("LINEAL_MAIL_FROM is not a valid address: {e}"))
        })?;
        // Klartext-SMTP: der Relay läuft auf demselben Host (Docker-Bridge), nicht im Internet.
        let transport =
            lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(&cfg.host)
                .port(cfg.port)
                .timeout(Some(std::time::Duration::from_secs(10)))
                .build();
        Ok(Self { transport, from })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: Mail) -> AppResult<()> {
        use lettre::AsyncTransport as _;
        let to: lettre::message::Mailbox = mail.to.parse().map_err(|_| {
            AppError::invalid_request("Invalid recipient address.")
                .with_details(serde_json::json!({ "field": "to" }))
        })?;
        let message = lettre::Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(lineal_domain::rules::text::sanitize_header_value(
                &mail.subject,
            ))
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(mail.body)
            .map_err(|e| AppError::internal(format!("could not build mail: {e}")))?;
        self.transport
            .send(message)
            .await
            .map_err(|e| AppError::internal(format!("SMTP delivery failed: {e}")))?;
        Ok(())
    }
}

/// Log- oder SMTP-Mailer je nach Konfiguration.
pub enum ConfiguredMailer {
    Log(LogMailer),
    Smtp(SmtpMailer),
}

impl ConfiguredMailer {
    pub fn is_smtp(&self) -> bool {
        matches!(self, Self::Smtp(_))
    }
}

#[async_trait]
impl Mailer for ConfiguredMailer {
    async fn send(&self, mail: Mail) -> AppResult<()> {
        match self {
            Self::Log(m) => m.send(mail).await,
            Self::Smtp(m) => m.send(mail).await,
        }
    }
}

/// SMTP, wenn `LINEAL_SMTP_HOST` gesetzt ist, sonst nur Log.
pub fn mailer_for(config: &Config) -> AppResult<ConfiguredMailer> {
    Ok(match &config.smtp {
        Some(smtp) => ConfiguredMailer::Smtp(SmtpMailer::new(smtp)?),
        None => ConfiguredMailer::Log(LogMailer),
    })
}

/// Verdrahtet die `App` mit allen Adaptern.
pub fn build_app(store: Arc<dyn Store>, config: &Config) -> Arc<App> {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Arc::new(App {
        store,
        blobs: Arc::new(LocalBlobStore::new(
            &config.blob_dir,
            &config.session_secret,
            &config.base_url,
            clock.clone(),
        )),
        clock,
        ids: Arc::new(UuidV7IdGen),
        search: Arc::new(NoSearch),
        hasher: Arc::new(Argon2Hasher::default()),
        mailer: Arc::new(mailer_for(config).unwrap_or_else(|e| {
            tracing::error!(error = %e, "SMTP mailer unavailable, mails are only logged");
            ConfiguredMailer::Log(LogMailer)
        })),
        config: AppConfig {
            base_url: config.base_url.clone(),
            files_base_url: config.files_base_url.clone(),
            session_secret: config.session_secret.clone(),
            blob_dir: config.blob_dir.clone(),
            allowed_origins: config.allowed_origins.clone(),
            trusted_proxies: config.trusted_proxies.clone(),
            workspace_creation: config.workspace_creation,
            instance_admins: config.instance_admins.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn argon2_roundtrip_with_salt_and_garbage_hash() {
        let h = Argon2Hasher::default();
        let a = h.hash("correct horse").unwrap();
        let b = h.hash("correct horse").unwrap();
        assert!(a.starts_with("$argon2id$"));
        assert_ne!(a, b, "salt must differ");
        assert!(h.verify("correct horse", &a).unwrap());
        assert!(!h.verify("wrong", &a).unwrap());
        assert!(!h.verify("x", "not-a-phc-string").unwrap());
    }

    #[test]
    fn clock_and_ids_are_real() {
        let before = Utc::now();
        assert!(SystemClock.now() >= before);
        let (a, b) = (UuidV7IdGen.new_uuid(), UuidV7IdGen.new_uuid());
        assert_eq!(a.get_version_num(), 7);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn build_app_wires_a_working_blob_store() {
        let cfg = Config::from_lookup(|k| (k == "LINEAL_DEV").then(|| "1".into())).unwrap();
        let app = build_app(Arc::new(lineal_testkit::FakeStore::new()), &cfg);
        let url = app
            .blobs
            .signed_put_url("k", "text/plain", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(url.starts_with("http://localhost:8080/uploads/"));
        assert!(app.hasher.hash("x").unwrap().starts_with("$argon2id$"));
    }

    /// Minimaler SMTP-Server: beantwortet einen Dialog und liefert die DATA-Zeilen.
    async fn fake_smtp(listener: tokio::net::TcpListener) -> Vec<String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (sock, _) = listener.accept().await.unwrap();
        let (r, mut w) = sock.into_split();
        let mut lines = BufReader::new(r).lines();
        w.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut data = Vec::new();
        let mut in_data = false;
        while let Some(line) = lines.next_line().await.unwrap() {
            if in_data {
                if line == "." {
                    in_data = false;
                    w.write_all(b"250 queued\r\n").await.unwrap();
                } else {
                    data.push(line);
                }
                continue;
            }
            let reply: &[u8] = match line
                .get(..4)
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str()
            {
                "EHLO" | "HELO" => b"250 fake\r\n",
                "DATA" => {
                    in_data = true;
                    b"354 go ahead\r\n"
                }
                "QUIT" => {
                    w.write_all(b"221 bye\r\n").await.unwrap();
                    break;
                }
                _ => b"250 ok\r\n",
            };
            w.write_all(reply).await.unwrap();
        }
        data
    }

    #[tokio::test]
    async fn smtp_mailer_delivers_plain_text_mail_to_the_relay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(fake_smtp(listener));
        let mailer = SmtpMailer::new(&SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "Lineal <lineal@example.org>".into(),
        })
        .unwrap();
        mailer
            .send(Mail {
                to: "bob@example.org".into(),
                subject: "You have been invited to Acme on Lineal".into(),
                body: "Hallo Bob,\nhttps://lineal.example/invite/abc".into(),
            })
            .await
            .unwrap();
        let data = server.await.unwrap().join("\n");
        assert!(data.contains("From: Lineal <lineal@example.org>"), "{data}");
        assert!(data.contains("To: bob@example.org"), "{data}");
        assert!(
            data.contains("Subject: You have been invited to Acme on Lineal"),
            "{data}"
        );
        assert!(
            data.contains("Content-Type: text/plain; charset=utf-8"),
            "{data}"
        );
        assert!(data.contains("https://lineal.example/invite/abc"), "{data}");
    }

    #[tokio::test]
    async fn smtp_mailer_rejects_invalid_recipients() {
        let mailer = SmtpMailer::new(&SmtpConfig {
            host: "127.0.0.1".into(),
            port: 9,
            from: "lineal@example.org".into(),
        })
        .unwrap();
        let err = mailer
            .send(Mail {
                to: "not an address".into(),
                subject: "x".into(),
                body: "y".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind, lineal_app::ErrorKind::InvalidRequest);
    }

    #[test]
    fn build_app_uses_smtp_only_when_configured() {
        let cfg = Config::from_lookup(|k| match k {
            "LINEAL_DEV" => Some("1".into()),
            "LINEAL_SMTP_HOST" => Some("127.0.0.1".into()),
            "LINEAL_MAIL_FROM" => Some("lineal@example.org".into()),
            _ => None,
        })
        .unwrap();
        assert!(mailer_for(&cfg).unwrap().is_smtp());
        let dev = Config::from_lookup(|k| (k == "LINEAL_DEV").then(|| "1".into())).unwrap();
        assert!(!mailer_for(&dev).unwrap().is_smtp());
    }
}
