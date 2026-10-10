//! Client-IP hinter einem Reverse-Proxy (`LINEAL_TRUSTED_PROXIES`).
//!
//! `X-Forwarded-For` wird nur ausgewertet, wenn die Peer-IP der TCP-Verbindung in einem
//! konfigurierten CIDR-Bereich liegt. Dann gilt der rechteste nicht vertrauenswürdige Eintrag
//! als Client (spätere Einträge hat ein vertrauenswürdiger Proxy angehängt, frühere kann der
//! Client beliebig fälschen). Das Ergebnis liegt als [`ClientIp`] in den Request-Extensions
//! und dient Rate-Limit und Logs.

use axum::extract::{ConnectInfo, Request};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Aufgelöste Client-IP einer Anfrage (Request-Extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

/// IP-Netz in CIDR-Schreibweise (`10.0.0.0/8`, `::1/128`; ohne Suffix = einzelne Adresse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Option<Self> {
        let (ip, prefix) = match text.trim().split_once('/') {
            Some((ip, p)) => (ip, Some(p.parse::<u8>().ok()?)),
            None => (text.trim(), None),
        };
        let addr: IpAddr = ip.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let (a, b, bits) = match (self.addr, normalize(ip)) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                (u128::from(u32::from(a)), u128::from(u32::from(b)), 32u32)
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => (u128::from(a), u128::from(b), 128u32),
            _ => return false,
        };
        let shift = bits - u32::from(self.prefix);
        shift >= 128 || (a >> shift) == (b >> shift)
    }
}

/// IPv4-mapped IPv6 (`::ffff:1.2.3.4`) wie IPv4 behandeln.
fn normalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Parst eine Liste; ungültige Einträge werden ignoriert (die Server-Konfiguration validiert vorher).
pub fn parse_list(entries: &[String]) -> Vec<Cidr> {
    entries.iter().filter_map(|e| Cidr::parse(e)).collect()
}

/// Parst alle `X-Forwarded-For`-Header strikt zu einer Liste (Reihenfolge der Header und
/// Einträge bleibt erhalten). Jeder Eintrag muss eine reine IP sein (nach Trimmen); Ports,
/// Klammern, Zone-IDs, `unknown`, obfuskierte Bezeichner und leere Einträge machen die ganze
/// Kette ungültig (`None`), damit kein vom Client gefälschter Eintrag weiter links gewählt wird.
fn parse_chain(headers: &HeaderMap) -> Option<Vec<IpAddr>> {
    let mut chain = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        for entry in value.to_str().ok()?.split(',') {
            chain.push(normalize(entry.trim().parse::<IpAddr>().ok()?));
        }
    }
    Some(chain)
}

/// `X-Forwarded-For` eines vertrauenswürdigen Proxys ist nicht lesbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidForwardedFor;

/// Client-IP aus Peer und Headern. Der `Forwarded`-Header (RFC 7239) wird bewusst nicht
/// ausgewertet, damit nicht zwei Quellen gemischt werden.
///
/// Ein ungültiger `X-Forwarded-For` hinter einem vertrauenswürdigen Proxy ergibt `Err`; die
/// Middleware lehnt den Request mit 400 ab. Alternativen wären die Proxy-IP (ein gemeinsamer
/// Bucket für alle Nutzer hinter dem Proxy: ein kaputter Header sperrt alle aus) oder eine
/// weiter links stehende, vom Client gefälschte IP; beides wird vermieden.
pub fn resolve(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted: &[Cidr],
) -> Result<IpAddr, InvalidForwardedFor> {
    let peer = normalize(peer);
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    if !is_trusted(peer) {
        return Ok(peer);
    }
    let chain = parse_chain(headers).ok_or(InvalidForwardedFor)?;
    Ok(chain
        .iter()
        .rev()
        .find(|ip| !is_trusted(**ip))
        .or(chain.first())
        .copied()
        .unwrap_or(peer))
}

/// Äußerste Middleware: setzt [`ClientIp`] (ohne `ConnectInfo`, z. B. in Router-Tests, nichts).
pub async fn middleware(
    axum::extract::State(trusted): axum::extract::State<Arc<Vec<Cidr>>>,
    mut req: Request,
    next: Next,
) -> Response {
    if let Some(ConnectInfo(peer)) = req.extensions().get::<ConnectInfo<SocketAddr>>().copied() {
        match resolve(peer.ip(), req.headers(), &trusted) {
            Ok(ip) => {
                req.extensions_mut().insert(ClientIp(ip));
            }
            Err(InvalidForwardedFor) => {
                return crate::error::ApiError(lineal_app::AppError::invalid_request(
                    "Invalid X-Forwarded-For header.",
                ))
                .into_response();
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn res(peer: IpAddr, headers: &HeaderMap, trusted: &[Cidr]) -> IpAddr {
        resolve(peer, headers, trusted).unwrap()
    }

    fn xff(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn cidr_parsing_and_matching() {
        let net = Cidr::parse("10.1.0.0/16").unwrap();
        assert!(net.contains(ip("10.1.200.3")));
        assert!(!net.contains(ip("10.2.0.1")));
        assert!(Cidr::parse("10.0.0.1").unwrap().contains(ip("10.0.0.1")));
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains(ip("8.8.8.8")));
        assert!(Cidr::parse("::1/128").unwrap().contains(ip("::1")));
        assert!(Cidr::parse("10.0.0.0/33").is_none());
        assert!(Cidr::parse("nope").is_none());
        assert!(
            Cidr::parse("10.0.0.0/8")
                .unwrap()
                .contains(ip("::ffff:10.1.2.3"))
        );
    }

    #[test]
    fn forged_header_from_untrusted_peer_is_ignored() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        let h = xff("1.2.3.4");
        assert_eq!(res(ip("203.0.113.9"), &h, &trusted), ip("203.0.113.9"));
        assert_eq!(res(ip("203.0.113.9"), &h, &[]), ip("203.0.113.9"));
    }

    #[test]
    fn rightmost_untrusted_entry_wins_behind_trusted_proxy() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        // Client fälscht "6.6.6.6"; der Proxy 10.0.0.5 hängt die echte Adresse an.
        let h = xff("6.6.6.6, 198.51.100.7, 10.0.0.9");
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
        // Ohne Header: Peer.
        assert_eq!(
            res(ip("10.0.0.5"), &HeaderMap::new(), &trusted),
            ip("10.0.0.5")
        );
    }

    fn multi(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append("x-forwarded-for", HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn multiple_headers_form_one_ordered_list() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        // Entspricht "6.6.6.6, 198.51.100.7, 10.0.0.9" in einem Header.
        let h = multi(&["6.6.6.6", "198.51.100.7, 10.0.0.9"]);
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
        // Der zweite Header (vom Proxy angehängt) zählt, nicht der erste.
        let h = multi(&["198.51.100.7", "10.0.0.9"]);
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
    }

    #[test]
    fn anything_but_a_plain_ip_is_rejected_behind_a_trusted_proxy() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        for bad in [
            "6.6.6.6, 1.2.3.4:5678",
            "6.6.6.6, [::1]:80",
            "6.6.6.6, [::1]",
            "6.6.6.6, unknown",
            "6.6.6.6, _hidden",
            "6.6.6.6, fe80::1%eth0",
            "6.6.6.6, , 198.51.100.7",
            "6.6.6.6,",
        ] {
            assert_eq!(
                resolve(ip("10.0.0.5"), &xff(bad), &trusted),
                Err(InvalidForwardedFor),
                "{bad}"
            );
            // Von einem nicht vertrauenswürdigen Peer wird der Header gar nicht gelesen.
            assert_eq!(
                res(ip("203.0.113.9"), &xff(bad), &trusted),
                ip("203.0.113.9")
            );
        }
        // Auch ein ungültiger Eintrag in einem zweiten Header macht alles ungültig.
        let h = multi(&["198.51.100.7", "unknown"]);
        assert!(resolve(ip("10.0.0.5"), &h, &trusted).is_err());
        // Whitespace wird getrimmt.
        assert_eq!(
            res(
                ip("10.0.0.5"),
                &xff("  198.51.100.7 ,\t10.0.0.9 "),
                &trusted
            ),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn mapped_ipv6_is_normalized_for_peer_and_entries() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        // Peer als mapped IPv6 gilt als 10.0.0.5 (vertrauenswürdig).
        let h = xff("198.51.100.7");
        assert_eq!(res(ip("::ffff:10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
        // Mapped Einträge werden abgezogen und das Ergebnis ist normalisiert.
        let h = xff("::ffff:198.51.100.7, ::ffff:10.0.0.9");
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
        // Alle vertrauenswürdig: linkester Eintrag.
        let h = xff("10.0.0.1, ::ffff:10.0.0.2");
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("10.0.0.1"));
    }

    #[test]
    fn forwarded_header_is_ignored() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        let mut h = HeaderMap::new();
        h.insert("forwarded", HeaderValue::from_static("for=6.6.6.6"));
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("10.0.0.5"));
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.7"));
        assert_eq!(res(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
    }
}
