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
use axum::response::Response;
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

/// Client-IP aus Peer und Headern.
pub fn resolve(peer: IpAddr, headers: &HeaderMap, trusted: &[Cidr]) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    if !is_trusted(peer) {
        return peer;
    }
    let chain: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .collect();
    chain
        .iter()
        .rev()
        .find(|ip| !is_trusted(**ip))
        .or(chain.first())
        .copied()
        .unwrap_or(peer)
}

/// Äußerste Middleware: setzt [`ClientIp`] (ohne `ConnectInfo`, z. B. in Router-Tests, nichts).
pub async fn middleware(
    axum::extract::State(trusted): axum::extract::State<Arc<Vec<Cidr>>>,
    mut req: Request,
    next: Next,
) -> Response {
    if let Some(ConnectInfo(peer)) = req.extensions().get::<ConnectInfo<SocketAddr>>().copied() {
        let ip = resolve(peer.ip(), req.headers(), &trusted);
        req.extensions_mut().insert(ClientIp(ip));
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
        assert_eq!(resolve(ip("203.0.113.9"), &h, &trusted), ip("203.0.113.9"));
        assert_eq!(resolve(ip("203.0.113.9"), &h, &[]), ip("203.0.113.9"));
    }

    #[test]
    fn rightmost_untrusted_entry_wins_behind_trusted_proxy() {
        let trusted = parse_list(&["10.0.0.0/8".to_owned()]);
        // Client fälscht "6.6.6.6"; der Proxy 10.0.0.5 hängt die echte Adresse an.
        let h = xff("6.6.6.6, 198.51.100.7, 10.0.0.9");
        assert_eq!(resolve(ip("10.0.0.5"), &h, &trusted), ip("198.51.100.7"));
        // Ohne Header: Peer.
        assert_eq!(
            resolve(ip("10.0.0.5"), &HeaderMap::new(), &trusted),
            ip("10.0.0.5")
        );
    }
}
