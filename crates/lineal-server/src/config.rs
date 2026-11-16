//! Konfiguration aus der Umgebung (12-Factor). WP03 erweitert.

use anyhow::{Context, Result, bail};
use lineal_app::WorkspaceCreation;
use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_DATABASE_URL: &str = "postgres://lineal:lineal@localhost:5440/lineal";
const DEFAULT_SESSION_SECRET: &str = "change-me-32-bytes-minimum-xxxxxxxx";

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// `LINEAL_APP_DB_PASSWORD`: Passwort der de-privilegierten Rolle `lineal_app` (LIN-61/S-09).
    /// Gesetzt => Server und Worker verbinden sich als `lineal_app` (nur DML), `migrate` legt die
    /// Rolle an. Ungesetzt/leer => Rueckfall auf die bisherige einzelne `DATABASE_URL`
    /// (Eigentuemer), damit Dev/Test und bestehende Installationen unveraendert laufen.
    pub app_db_password: Option<String>,
    pub base_url: String,
    /// `LINEAL_FILES_BASE_URL`: eigene, cookielose Origin für die Inline-Ansicht von HTML-Anhängen
    /// (Weg B, `docs/superpowers/specs/2026-11-13-html-anhang-tab.md`). `None` = Feature aus, HTML
    /// wird überall als Download ausgeliefert. Muss sich von `base_url` unterscheiden (sonst keine
    /// Isolation). Normalisiert ohne Slash am Ende.
    pub files_base_url: Option<String>,
    pub session_secret: String,
    pub blob_dir: PathBuf,
    pub bind: SocketAddr,
    /// Entwicklungsmodus (GraphiQL, lockere Defaults).
    pub dev: bool,
    pub db_max_connections: u32,
    /// `LINEAL_DB_STATEMENT_TIMEOUT_MS`: `statement_timeout` (ms) des Request-Pools (S-19). `0`
    /// schaltet es ab; Default 30000. Migrationen, Import und Worker laufen ohne Grenze.
    pub db_statement_timeout_ms: Option<u32>,
    /// `LINEAL_ALLOWED_ORIGINS` (Komma-Liste); leer = Origin von `base_url`.
    pub allowed_origins: Vec<String>,
    /// `LINEAL_TRUSTED_PROXIES` (CIDR-Liste); leer = `X-Forwarded-For` wird ignoriert.
    pub trusted_proxies: Vec<String>,
    /// `LINEAL_WORKSPACE_CREATION=disabled|admins|all`; Default `all` im Dev-Modus, sonst
    /// `disabled`. Gilt für GraphQL `createWorkspace`, nicht für das CLI.
    pub workspace_creation: WorkspaceCreation,
    /// `LINEAL_INSTANCE_ADMINS` (Komma-Liste von User-UUIDs) für `admins`.
    pub instance_admins: Vec<uuid::Uuid>,
    /// `LINEAL_TOTP_ENFORCE_OWNERS` (Default `true`): erzwingt TOTP für Org-Owner
    /// (Schonfrist, dann harte Sperre, LIN-70). `0`/`false`/`no` schaltet es ab.
    pub totp_enforce_owners: bool,
    /// SMTP-Relay (`LINEAL_SMTP_HOST`, `LINEAL_SMTP_PORT`, `LINEAL_MAIL_FROM`); `None` = Mails
    /// nur ins Log.
    pub smtp: Option<SmtpConfig>,
    /// Backend fuer die Verschluesselung der TOTP-Secrets (LIN-70). Default `app`
    /// (Schluessel aus `session_secret`); `openbao` nutzt OpenBAO Transit.
    pub totp_key_backend: TotpKeyBackend,
    /// OpenBAO-Zugang; nur gesetzt, wenn `totp_key_backend = OpenBao`.
    pub openbao: Option<OpenBaoConfig>,
}

/// Ziel für ausgehende Mails: ein Relay ohne Anmeldung (z. B. Postfix auf dem Host).
/// Schluessel-Backend fuer die TOTP-Secrets (LIN-70).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpKeyBackend {
    App,
    OpenBao,
}

/// Zugang zu OpenBAO Transit.
#[derive(Debug, Clone)]
pub struct OpenBaoConfig {
    /// Basis-URL, z. B. `https://bao.internal:8200` (ohne Slash am Ende).
    pub addr: String,
    /// Vault-/OpenBAO-Token (`X-Vault-Token`).
    pub token: String,
    /// Name des Transit-Schluessels, z. B. `lineal-totp`.
    pub transit_key: String,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// Absender, z. B. `Lineal <lineal@example.org>`.
    pub from: String,
}

fn smtp(get: &impl Fn(&str) -> Option<String>) -> Result<Option<SmtpConfig>> {
    let Some(host) = get("LINEAL_SMTP_HOST").filter(|h| !h.trim().is_empty()) else {
        return Ok(None);
    };
    let port = get("LINEAL_SMTP_PORT")
        .map(|v| v.parse())
        .transpose()
        .context("LINEAL_SMTP_PORT must be a port number")?
        .unwrap_or(25);
    let from = get("LINEAL_MAIL_FROM")
        .filter(|f| !f.trim().is_empty())
        .context("LINEAL_MAIL_FROM must be set when LINEAL_SMTP_HOST is set")?;
    from.parse::<lettre::message::Mailbox>()
        .with_context(|| format!("LINEAL_MAIL_FROM: {from:?} is not a valid address"))?;
    Ok(Some(SmtpConfig {
        host: host.trim().to_owned(),
        port,
        from,
    }))
}

fn csv(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().trim_end_matches('/').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 256 Bit Zufall (zwei UUIDv4) als Hex – nur für den Dev-Modus ohne gesetztes Secret.
fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn is_dev(v: Option<String>) -> bool {
    v.is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

/// `LINEAL_DEV` aus der Umgebung (ohne die übrige Konfiguration zu prüfen).
pub fn dev_from_env() -> bool {
    is_dev(std::env::var("LINEAL_DEV").ok())
}

/// LIN-65 N10: Loest einen Konfigurationswert auf. Ist `KEY` ungesetzt/leer, aber `KEY_FILE`
/// gesetzt, wird der Dateiinhalt (ohne abschliessende Zeilenumbrueche) verwendet. So lassen sich
/// Geheimnisse per Docker-/Compose-`secrets:` als Datei statt als Umgebungsvariable uebergeben
/// (letztere sind in jedem `docker inspect` sichtbar). Die direkte Variable hat Vorrang.
fn env_or_file<E, R>(key: &str, env: &E, read: &R) -> Option<String>
where
    E: Fn(&str) -> Option<String>,
    R: Fn(&str) -> std::io::Result<String>,
{
    if let Some(v) = env(key).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let path = env(&format!("{key}_FILE")).filter(|p| !p.is_empty())?;
    match read(&path) {
        Ok(s) => Some(s.trim_end_matches(['\n', '\r']).to_string()),
        Err(e) => {
            eprintln!("config: {key}_FILE ({path}) konnte nicht gelesen werden: {e}");
            None
        }
    }
}

impl Config {
    /// Liest die Konfiguration aus Umgebungsvariablen (siehe `.env.example`), mit `*_FILE`-Fallback
    /// fuer Geheimnisse (siehe [`env_or_file`]).
    pub fn from_env() -> Result<Self> {
        let env = |k: &str| std::env::var(k).ok();
        let read = |p: &str| std::fs::read_to_string(p);
        Self::from_lookup(|k| env_or_file(k, &env, &read))
    }

    /// Wie [`Config::from_env`], mit austauschbarer Quelle (Tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let dev = is_dev(get("LINEAL_DEV"));
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
        let allowed_origins = csv(get("LINEAL_ALLOWED_ORIGINS"));
        let trusted_proxies = csv(get("LINEAL_TRUSTED_PROXIES"));
        for cidr in &trusted_proxies {
            if lineal_api::client_ip::Cidr::parse(cidr).is_none() {
                bail!("LINEAL_TRUSTED_PROXIES: invalid CIDR {cidr:?}");
            }
        }
        let workspace_creation = match get("LINEAL_WORKSPACE_CREATION") {
            Some(v) if !v.trim().is_empty() => WorkspaceCreation::parse(&v)
                .context("LINEAL_WORKSPACE_CREATION must be disabled, admins or all")?,
            _ if dev => WorkspaceCreation::All,
            _ => WorkspaceCreation::Disabled,
        };
        let instance_admins = csv(get("LINEAL_INSTANCE_ADMINS"))
            .into_iter()
            .map(|id| {
                uuid::Uuid::parse_str(&id)
                    .with_context(|| format!("LINEAL_INSTANCE_ADMINS: {id:?} is not a user UUID"))
            })
            .collect::<Result<Vec<_>>>()?;
        let base_url = get("LINEAL_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into());
        // L-12: Der Dev-Modus lockert Annahmen (Beispiel-Secret zulässig, CORS auf localhost,
        // Workspace-Anlage für alle). Eine https-Basis-URL signalisiert dagegen eine echte
        // Installation. Beides zusammen ist fast immer eine Fehlkonfiguration, die wie Prod
        // aussähe, aber mit öffentlich bekannten Annahmen liefe – daher hart abbrechen.
        if dev
            && base_url
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("https://")
        {
            bail!(
                "LINEAL_DEV must not be combined with an https LINEAL_BASE_URL (dev mode relaxes secret/CORS/workspace checks)"
            );
        }
        // Weg B: eigene Datei-Origin für die Inline-Ansicht von HTML-Anhängen. Muss sich von der
        // App-Origin unterscheiden, sonst gäbe es keine Cookie-/Script-Isolation.
        let files_base_url = match get("LINEAL_FILES_BASE_URL") {
            Some(v) if !v.trim().is_empty() => {
                let normalized = v.trim().trim_end_matches('/').to_owned();
                if normalized == base_url.trim().trim_end_matches('/') {
                    bail!(
                        "LINEAL_FILES_BASE_URL must differ from LINEAL_BASE_URL (it is a separate cookieless origin for HTML attachments)"
                    );
                }
                Some(normalized)
            }
            _ => None,
        };
        let smtp = smtp(&get)?;
        let totp_key_backend = match get("LINEAL_TOTP_KEY_BACKEND").as_deref() {
            Some("openbao") => TotpKeyBackend::OpenBao,
            _ => TotpKeyBackend::App,
        };
        let openbao = if totp_key_backend == TotpKeyBackend::OpenBao {
            Some(OpenBaoConfig {
                addr: get("LINEAL_OPENBAO_ADDR")
                    .map(|a| a.trim_end_matches('/').to_string())
                    .context("LINEAL_OPENBAO_ADDR is required for the openbao backend")?,
                token: get("LINEAL_OPENBAO_TOKEN")
                    .context("LINEAL_OPENBAO_TOKEN is required for the openbao backend")?,
                transit_key: get("LINEAL_OPENBAO_TRANSIT_KEY")
                    .context("LINEAL_OPENBAO_TRANSIT_KEY is required for the openbao backend")?,
            })
        } else {
            None
        };
        Ok(Self {
            database_url: get("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.into()),
            app_db_password: get("LINEAL_APP_DB_PASSWORD").filter(|s| !s.trim().is_empty()),
            base_url,
            files_base_url,
            session_secret,
            blob_dir: get("LINEAL_BLOB_DIR")
                .map_or_else(|| PathBuf::from("./var/blobs"), PathBuf::from),
            bind: get("LINEAL_BIND")
                .unwrap_or_else(|| "127.0.0.1:8080".into())
                .parse()
                .context("LINEAL_BIND must be host:port")?,
            dev,
            allowed_origins,
            trusted_proxies,
            workspace_creation,
            instance_admins,
            totp_enforce_owners: !matches!(
                get("LINEAL_TOTP_ENFORCE_OWNERS").as_deref(),
                Some("0") | Some("false") | Some("no")
            ),
            totp_key_backend,
            openbao,
            smtp,
            db_max_connections: get("LINEAL_DB_MAX_CONNECTIONS")
                .map(|v| v.parse())
                .transpose()
                .context("LINEAL_DB_MAX_CONNECTIONS must be a number")?
                .unwrap_or(10),
            db_statement_timeout_ms: {
                let ms = get("LINEAL_DB_STATEMENT_TIMEOUT_MS")
                    .map(|v| v.parse())
                    .transpose()
                    .context("LINEAL_DB_STATEMENT_TIMEOUT_MS must be a number")?
                    .unwrap_or(30_000u32);
                (ms > 0).then_some(ms)
            },
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

    /// LIN-65 N10: `KEY_FILE` liefert den Dateiinhalt (ohne Zeilenumbruch); die direkte Variable
    /// hat Vorrang und wird ohne Dateizugriff genommen.
    #[test]
    fn env_or_file_reads_file_fallback_and_prefers_direct() {
        let env = |k: &str| (k == "LINEAL_SESSION_SECRET_FILE").then(|| "/run/secrets/sess".into());
        let read = |p: &str| {
            assert_eq!(p, "/run/secrets/sess");
            Ok("a-real-secret-with-at-least-32-bytes!\n".to_string())
        };
        assert_eq!(
            env_or_file("LINEAL_SESSION_SECRET", &env, &read).as_deref(),
            Some("a-real-secret-with-at-least-32-bytes!")
        );

        let env2 = |k: &str| match k {
            "LINEAL_SESSION_SECRET" => Some("direct-value-at-least-32-bytes-long!".into()),
            "LINEAL_SESSION_SECRET_FILE" => Some("/unused".into()),
            _ => None,
        };
        let read_panic = |_: &str| -> std::io::Result<String> {
            panic!("Datei darf bei gesetzter Variable nicht gelesen werden")
        };
        assert_eq!(
            env_or_file("LINEAL_SESSION_SECRET", &env2, &read_panic).as_deref(),
            Some("direct-value-at-least-32-bytes-long!")
        );

        // Ungesetzt (weder Variable noch _FILE) -> None.
        let none = |_: &str| None;
        let read_none = |_: &str| -> std::io::Result<String> { unreachable!() };
        assert!(env_or_file("LINEAL_SESSION_SECRET", &none, &read_none).is_none());
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
    fn files_base_url_is_optional_and_distinct() {
        // Ungesetzt -> None (Feature aus, HTML überall als Download).
        let dev = |k: &str| (k == "LINEAL_DEV").then(|| "1".into());
        assert_eq!(Config::from_lookup(dev).unwrap().files_base_url, None);
        // Gesetzt -> normalisiert ohne Slash am Ende.
        let set = |k: &str| match k {
            "LINEAL_SESSION_SECRET" => Some("a-real-secret-with-at-least-32-bytes!".into()),
            "LINEAL_BASE_URL" => Some("https://lineal.example/".into()),
            "LINEAL_FILES_BASE_URL" => Some("https://lineal-files.example/".into()),
            _ => None,
        };
        assert_eq!(
            Config::from_lookup(set).unwrap().files_base_url.as_deref(),
            Some("https://lineal-files.example")
        );
        // Gleich wie base_url -> Fehler: ohne getrennte Origin keine Isolation.
        let same = |k: &str| match k {
            "LINEAL_SESSION_SECRET" => Some("a-real-secret-with-at-least-32-bytes!".into()),
            "LINEAL_BASE_URL" => Some("https://lineal.example".into()),
            "LINEAL_FILES_BASE_URL" => Some("https://lineal.example/".into()),
            _ => None,
        };
        assert!(Config::from_lookup(same).is_err());
    }

    #[test]
    fn dev_mode_is_rejected_behind_https() {
        // L-12: Der Dev-Modus akzeptiert u. a. das Beispiel-Secret und lockert CORS/Workspace-
        // Anlage. Zusammen mit einer https-Basis-URL sähe das wie eine echte Installation aus,
        // liefe aber mit öffentlich bekannten Annahmen. Der Start muss abbrechen.
        let dev_https = |k: &str| match k {
            "LINEAL_DEV" => Some("1".into()),
            "LINEAL_BASE_URL" => Some("https://lineal.example".into()),
            _ => None,
        };
        assert!(Config::from_lookup(dev_https).is_err());
        // Dev mit http (Default) bleibt erlaubt.
        let dev_http = |k: &str| (k == "LINEAL_DEV").then(|| "1".into());
        assert!(Config::from_lookup(dev_http).is_ok());
    }

    #[test]
    fn smtp_is_optional_and_needs_a_sender() {
        let with =
            |host: Option<&'static str>, port: Option<&'static str>, from: Option<&'static str>| {
                Config::from_lookup(move |k| match k {
                    "LINEAL_DEV" => Some("1".into()),
                    "LINEAL_SMTP_HOST" => host.map(Into::into),
                    "LINEAL_SMTP_PORT" => port.map(Into::into),
                    "LINEAL_MAIL_FROM" => from.map(Into::into),
                    _ => None,
                })
            };
        // Ohne Host: kein Versand (Log-Mailer).
        assert!(with(None, None, None).unwrap().smtp.is_none());
        let smtp = with(
            Some("mail.local"),
            None,
            Some("Lineal <lineal@example.org>"),
        )
        .unwrap()
        .smtp
        .unwrap();
        assert_eq!(
            (smtp.host.as_str(), smtp.port, smtp.from.as_str()),
            ("mail.local", 25, "Lineal <lineal@example.org>")
        );
        assert_eq!(
            with(Some("mail.local"), Some("2525"), Some("lineal@example.org"))
                .unwrap()
                .smtp
                .unwrap()
                .port,
            2525
        );
        assert!(
            with(Some("mail.local"), None, None).is_err(),
            "sender required"
        );
        assert!(with(Some("mail.local"), None, Some("not an address")).is_err());
        assert!(with(Some("mail.local"), Some("x"), Some("lineal@example.org")).is_err());
    }

    #[test]
    fn origins_and_trusted_proxies_are_parsed_and_validated() {
        let with = |origins: &'static str, proxies: &'static str| {
            Config::from_lookup(move |k| match k {
                "LINEAL_DEV" => Some("1".into()),
                "LINEAL_ALLOWED_ORIGINS" => Some(origins.into()),
                "LINEAL_TRUSTED_PROXIES" => Some(proxies.into()),
                _ => None,
            })
        };
        let cfg = with("https://a.example/, https://b.example", "10.0.0.0/8, ::1").unwrap();
        assert_eq!(
            cfg.allowed_origins,
            ["https://a.example", "https://b.example"]
        );
        assert_eq!(cfg.trusted_proxies, ["10.0.0.0/8", "::1"]);
        assert!(with("", "").unwrap().trusted_proxies.is_empty());
        assert!(with("", "10.0.0.0/99").is_err());
    }

    #[test]
    fn workspace_creation_defaults_and_admins() {
        let with = |dev: bool, mode: Option<&str>, admins: Option<&str>| {
            let (mode, admins) = (mode.map(str::to_owned), admins.map(str::to_owned));
            Config::from_lookup(move |k| match k {
                "LINEAL_DEV" => dev.then(|| "1".into()),
                "LINEAL_SESSION_SECRET" => Some("a-real-secret-with-at-least-32-bytes!".into()),
                "LINEAL_WORKSPACE_CREATION" => mode.clone(),
                "LINEAL_INSTANCE_ADMINS" => admins.clone(),
                _ => None,
            })
        };
        assert_eq!(
            with(true, None, None).unwrap().workspace_creation,
            WorkspaceCreation::All
        );
        assert_eq!(
            with(false, None, None).unwrap().workspace_creation,
            WorkspaceCreation::Disabled
        );
        assert_eq!(
            with(false, Some("All"), None).unwrap().workspace_creation,
            WorkspaceCreation::All
        );
        let id = "0190f1c2-7b1e-7c3a-9d2e-5a4b6c7d8e9f";
        let cfg = with(false, Some("admins"), Some(&format!(" {id} ,"))).unwrap();
        assert_eq!(cfg.workspace_creation, WorkspaceCreation::Admins);
        assert_eq!(cfg.instance_admins, [uuid::Uuid::parse_str(id).unwrap()]);
        assert!(with(false, Some("maybe"), None).is_err());
        assert!(with(false, Some("admins"), Some("alice@acme.example")).is_err());
        assert!(with(false, Some("admins"), Some("not-a-uuid")).is_err());
    }

    #[test]
    fn app_db_password_is_optional_and_trimmed() {
        // Ungesetzt => None (Rueckfall auf die Eigentuemer-URL, LIN-61/S-09).
        let dev = |k: &str| (k == "LINEAL_DEV").then(|| "1".into());
        assert_eq!(Config::from_lookup(dev).unwrap().app_db_password, None);
        // Leer/nur Whitespace => None.
        let empty = |k: &str| match k {
            "LINEAL_DEV" => Some("1".into()),
            "LINEAL_APP_DB_PASSWORD" => Some("  ".into()),
            _ => None,
        };
        assert_eq!(Config::from_lookup(empty).unwrap().app_db_password, None);
        // Gesetzt => uebernommen.
        let set = |k: &str| match k {
            "LINEAL_DEV" => Some("1".into()),
            "LINEAL_APP_DB_PASSWORD" => Some("s3cret".into()),
            _ => None,
        };
        assert_eq!(
            Config::from_lookup(set).unwrap().app_db_password.as_deref(),
            Some("s3cret")
        );
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
