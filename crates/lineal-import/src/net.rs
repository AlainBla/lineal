//! Abgesicherte Downloads (docs/13 §8): nur https, nur Hosts der Quelle (Allowlist), keine
//! privaten/lokalen IP-Adressen (SSRF-Schutz), Redirects werden jedes Mal neu geprüft, Größenlimit
//! beim Streamen, Content-Type-Allowlist. Die geprüfte IP wird für die Verbindung festgeschrieben
//! (kein DNS-Rebinding zwischen Prüfung und Verbindung).

use crate::port::{Downloaded, SourceError, SourceResult};
use futures_util::StreamExt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use url::Url;

/// Standard-Größenlimit je Datei (wie Uploads: 50 MB).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// Erlaubte Content-Types (Präfix-Vergleich auf den Typ ohne Parameter). Bewusst ohne
/// `text/html` und `image/svg+xml` (aktive Inhalte).
pub const ALLOWED_CONTENT_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/heic",
    "application/pdf",
    "text/plain",
    "text/csv",
    "text/markdown",
    "application/json",
    "application/zip",
    "application/gzip",
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "audio/mpeg",
    "audio/mp4",
    "audio/wav",
    "audio/webm",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/octet-stream",
];

/// Regeln für Downloads.
#[derive(Debug, Clone)]
pub struct UrlPolicy {
    /// Erlaubte Hosts (exakt oder Subdomain davon).
    pub allowed_hosts: Vec<String>,
    pub max_bytes: u64,
    pub allowed_content_types: Vec<String>,
    /// Nur für Tests gegen einen lokalen Mock-Server: erlaubt `http` und Loopback-Adressen
    /// (Allowlist bleibt wirksam). In Produktion immer `false`.
    pub allow_local_for_tests: bool,
    pub timeout: Duration,
}

impl UrlPolicy {
    /// Policy für Linear-Uploads.
    pub fn linear() -> Self {
        Self::for_hosts(vec!["uploads.linear.app".into()])
    }

    /// Policy für beliebige Hosts-Allowlist (https, Standardlimits).
    pub fn for_hosts(hosts: Vec<String>) -> Self {
        Self {
            allowed_hosts: hosts,
            max_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            allowed_content_types: ALLOWED_CONTENT_TYPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            allow_local_for_tests: false,
            timeout: Duration::from_secs(60),
        }
    }

    fn host_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allowed_hosts.iter().any(|a| {
            let a = a.to_ascii_lowercase();
            host == a || host.ends_with(&format!(".{a}"))
        })
    }

    /// Syntaktische Prüfung (Schema, Host-Allowlist, keine Zugangsdaten in der URL).
    pub fn check_url(&self, raw: &str) -> SourceResult<Url> {
        let rej = |m: &str| SourceError::Rejected(m.to_owned());
        let url = Url::parse(raw).map_err(|_| rej("not a valid URL"))?;
        let scheme_ok =
            url.scheme() == "https" || (self.allow_local_for_tests && url.scheme() == "http");
        if !scheme_ok {
            return Err(rej("only https URLs are allowed"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(rej("URLs with credentials are not allowed"));
        }
        let host = url.host_str().ok_or_else(|| rej("URL has no host"))?;
        if !self.host_allowed(host) {
            return Err(rej("host is not an allowed host of the source"));
        }
        if !self.allow_local_for_tests {
            if let Some(port) = url.port() {
                if port != 443 {
                    return Err(rej("only the default https port is allowed"));
                }
            }
        }
        Ok(url)
    }

    fn content_type_ok(&self, ct: &str) -> bool {
        let base = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        self.allowed_content_types.contains(&base)
    }

    /// Löst den Host auf und lehnt ab, wenn irgendeine Adresse privat/lokal ist.
    async fn resolve(&self, url: &Url) -> SourceResult<(String, Vec<SocketAddr>)> {
        let host = url.host_str().unwrap_or_default().to_owned();
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs: Vec<SocketAddr> = if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
        {
            vec![SocketAddr::new(ip, port)]
        } else {
            tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|_| SourceError::Transport("host cannot be resolved".into()))?
                .collect()
        };
        if addrs.is_empty() {
            return Err(SourceError::Transport("host cannot be resolved".into()));
        }
        if !self.allow_local_for_tests && addrs.iter().any(|a| is_forbidden_ip(a.ip())) {
            return Err(SourceError::Rejected(
                "host resolves to a private or local address".into(),
            ));
        }
        Ok((host, addrs))
    }

    /// Größe per `HEAD` (kein Inhalt wird geladen). Dieselben Prüfungen wie `fetch`; `None`, wenn
    /// der Server keine Länge nennt.
    pub async fn head_size(
        &self,
        raw_url: &str,
        auth_header: Option<&str>,
    ) -> SourceResult<Option<u64>> {
        let url = self.check_url(raw_url)?;
        let (host, addrs) = self.resolve(&url).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|_| SourceError::Transport("http client setup failed".into()))?;
        let mut req = client.head(url);
        if let Some(h) = auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, h);
        }
        let resp = req
            .send()
            .await
            .map_err(|_| SourceError::Transport("probe request failed".into()))?;
        let status = resp.status();
        if status.is_redirection() {
            return Err(SourceError::Rejected("redirects are not followed".into()));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SourceError::Auth);
        }
        if !status.is_success() {
            return Err(SourceError::Transport(format!(
                "probe failed with HTTP {}",
                status.as_u16()
            )));
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !self.content_type_ok(ct) {
            return Err(SourceError::Rejected("content type is not allowed".into()));
        }
        let len = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if len.is_some_and(|l| l > self.max_bytes) {
            return Err(SourceError::Rejected("file is too large".into()));
        }
        Ok(len)
    }

    /// Lädt die Datei (siehe Moduldoku). Fehlertexte enthalten weder Token noch Antwortinhalt.
    pub async fn fetch(
        &self,
        raw_url: &str,
        auth_header: Option<&str>,
    ) -> SourceResult<Downloaded> {
        let url = self.check_url(raw_url)?;
        {
            let (host, addrs) = self.resolve(&url).await?;
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(self.timeout)
                .resolve_to_addrs(&host, &addrs)
                .build()
                .map_err(|_| SourceError::Transport("http client setup failed".into()))?;
            let mut req = client.get(url.clone());
            // Zugangsdaten nur an Hosts der Allowlist (hier immer der Fall) und nie bei Redirects auf
            // andere Hosts: `check_url` lässt ohnehin nur Allowlist-Hosts zu.
            if let Some(h) = auth_header {
                req = req.header(reqwest::header::AUTHORIZATION, h);
            }
            let resp = req
                .send()
                .await
                .map_err(|_| SourceError::Transport("download request failed".into()))?;
            let status = resp.status();
            if status.is_redirection() {
                // Redirects werden nie gefolgt: weder Token noch Server-Rechte dürfen über eine
                // Weiterleitung an andere Ziele gelangen.
                return Err(SourceError::Rejected("redirects are not followed".into()));
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(SourceError::Auth);
            }
            if !status.is_success() {
                return Err(SourceError::Transport(format!(
                    "download failed with HTTP {}",
                    status.as_u16()
                )));
            }
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            if !self.content_type_ok(&content_type) {
                return Err(SourceError::Rejected("content type is not allowed".into()));
            }
            if resp.content_length().is_some_and(|l| l > self.max_bytes) {
                return Err(SourceError::Rejected("file is too large".into()));
            }
            let mut bytes: Vec<u8> = Vec::new();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|_| SourceError::Transport("download interrupted".into()))?;
                if (bytes.len() as u64).saturating_add(chunk.len() as u64) > self.max_bytes {
                    return Err(SourceError::Rejected("file is too large".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Downloaded {
                bytes,
                content_type: content_type
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase(),
            })
        }
    }
}

/// Private, lokale, Link-Local-, Multicast- und sonstige nicht öffentliche Adressen.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4);
            }
            forbidden_v6(v6)
        }
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT 100.64.0.0/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
        || (o[0] == 192 && o[1] == 0 && o[2] == 2) // TEST-NET-1
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // Benchmarking
        || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
        || (o[0] == 203 && o[1] == 0 && o[2] == 113) // TEST-NET-3
        || o[0] >= 240 // reserviert
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (seg[0] & 0xffc0) == 0xfe80 // Link-Local fe80::/10
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // Dokumentation
        || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.5",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                is_forbidden_ip(ip.parse().unwrap()),
                "{ip} must be rejected"
            );
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(
                !is_forbidden_ip(ip.parse().unwrap()),
                "{ip} must be allowed"
            );
        }
    }

    #[test]
    fn default_policy_rejects_unsafe_urls() {
        let p = UrlPolicy::linear();
        assert!(p.check_url("https://uploads.linear.app/a/b.png").is_ok());
        assert!(p.check_url("https://cdn.uploads.linear.app/a.png").is_ok());
        for bad in [
            "http://uploads.linear.app/a.png",
            "https://evil.example.com/a.png",
            "https://uploads.linear.app.evil.com/a.png",
            "https://127.0.0.1/a.png",
            "https://[::1]/a.png",
            "https://169.254.169.254/latest/meta-data",
            "https://user:pw@uploads.linear.app/a.png",
            "https://uploads.linear.app:8443/a.png",
            "file:///etc/passwd",
            "ftp://uploads.linear.app/a",
            "not a url",
        ] {
            assert!(p.check_url(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn content_type_allowlist() {
        let p = UrlPolicy::linear();
        assert!(p.content_type_ok("image/png"));
        assert!(p.content_type_ok("image/png; charset=binary"));
        assert!(!p.content_type_ok("text/html"));
        assert!(!p.content_type_ok("image/svg+xml"));
        assert!(!p.content_type_ok(""));
    }

    #[tokio::test]
    async fn literal_private_ip_is_rejected_after_resolution() {
        let mut p = UrlPolicy::for_hosts(vec!["127.0.0.1".into()]);
        p.allow_local_for_tests = false;
        // Host steht in der Allowlist, ist aber eine IP-Adresse im Loopback-Bereich.
        let e = p.fetch("https://127.0.0.1/x.png", None).await.unwrap_err();
        assert!(matches!(e, SourceError::Rejected(_)));
    }
}
