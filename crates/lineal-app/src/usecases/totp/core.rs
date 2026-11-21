//! Reine TOTP-/HOTP-Mechanik nach RFC 4226 / RFC 6238 (HMAC-SHA1).
//!
//! Alles hier ist zustandslos und seiteneffektfrei: Secret erzeugen, otpauth-URI
//! bauen, Codes berechnen/prüfen. Verifikation ist konstantzeitig und gibt den
//! getroffenen Zeitschritt zurück, damit der Aufrufer Code-Replay (RFC 6238 §5.2)
//! über `last_used_step` ausschließen kann.

use hmac::{Hmac, KeyInit, Mac};
use rand::RngCore;
use sha1::Sha1;
use subtle::ConstantTimeEq;

type HmacSha1 = Hmac<Sha1>;

/// Länge des TOTP-Secrets in Bytes (RFC-6238-üblich: 160 Bit).
pub const SECRET_LEN: usize = 20;
/// Standard-Stellenzahl der Codes.
pub const DIGITS: u32 = 6;
/// Fensterbreite in Sekunden.
pub const STEP_SECS: i64 = 30;
/// Akzeptierte Nachbarschritte in jede Richtung (±1 Schritt Toleranz).
pub const SKEW: i64 = 1;

/// Erzeugt ein frisches 20-Byte-Secret aus dem CSPRNG des Systems.
pub fn generate_secret() -> [u8; SECRET_LEN] {
    let mut out = [0u8; SECRET_LEN];
    rand::rngs::OsRng.fill_bytes(&mut out);
    out
}

/// Base32 (RFC 4648, ohne Padding, Großbuchstaben) — Format für QR/manuelle Eingabe.
pub fn base32_encode(secret: &[u8]) -> String {
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, secret)
}

/// Dekodiert Base32 (RFC 4648, Groß-/Kleinschreibung egal) zurück zum Rohsecret.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    base32::decode(
        base32::Alphabet::Rfc4648 { padding: false },
        &s.to_uppercase(),
    )
}

/// `otpauth://`-URI für den QR-Code. `issuer`/`account` werden prozent-kodiert.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    let label = format!("{}:{}", percent_encode(issuer), percent_encode(account));
    format!(
        "otpauth://totp/{label}?secret={secret}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        secret = base32_encode(secret),
        issuer = percent_encode(issuer),
    )
}

/// Minimale Prozent-Kodierung für die URI-Bestandteile (Doppelpunkt, Slash,
/// Fragezeichen, Leerzeichen, Prozent, Ampersand).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// HOTP (RFC 4226) mit dynamischer Trunkierung.
fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC akzeptiert jede Schlüssellänge");
    mac.update(&counter.to_be_bytes());
    let hs = mac.finalize().into_bytes();
    let offset = (hs[19] & 0x0f) as usize;
    let bin = (u32::from(hs[offset] & 0x7f) << 24)
        | (u32::from(hs[offset + 1]) << 16)
        | (u32::from(hs[offset + 2]) << 8)
        | u32::from(hs[offset + 3]);
    bin % 10u32.pow(digits)
}

/// Zeitschritt für einen Unix-Zeitstempel (Sekunden).
pub fn step_at(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(STEP_SECS)
}

/// Berechnet den Code für einen gegebenen Zeitschritt, links mit Nullen aufgefüllt.
pub fn code_for_step(secret: &[u8], step: i64, digits: u32) -> String {
    let code = hotp(secret, step as u64, digits);
    format!("{code:0width$}", width = digits as usize)
}

/// Aktueller TOTP-Code (für Tests/Server-seitige Erzeugung).
pub fn code_now(secret: &[u8], unix_secs: i64) -> String {
    code_for_step(secret, step_at(unix_secs), DIGITS)
}

/// Prüft `input` gegen das Fenster um `unix_secs` (±`SKEW`). Gibt den getroffenen
/// Zeitschritt zurück (für den Replay-Schutz) oder `None`. Nur Schritte **größer**
/// als `min_step` werden akzeptiert, falls gesetzt — so gilt ein benutzter Code
/// nicht erneut. Vergleich konstantzeitig.
pub fn verify(secret: &[u8], input: &str, unix_secs: i64, min_step: Option<i64>) -> Option<i64> {
    let input = input.trim();
    if input.len() != DIGITS as usize || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let base = step_at(unix_secs);
    let mut matched: Option<i64> = None;
    // Alle Kandidaten durchlaufen (kein Early-Return), damit die Laufzeit nicht vom
    // Treffer-Index abhängt.
    for step in (base - SKEW)..=(base + SKEW) {
        let candidate = code_for_step(secret, step, DIGITS);
        let eq = candidate.as_bytes().ct_eq(input.as_bytes()).into();
        let fresh = min_step.is_none_or(|m| step > m);
        if eq && fresh {
            matched = Some(step);
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 Appendix B nutzt das ASCII-Secret "12345678901234567890" (20 Byte),
    // SHA1, 8 Stellen. Wir prüfen die 8-stellige HOTP-Trunkierung direkt.
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    fn code8(unix: i64) -> String {
        code_for_step(RFC_SECRET, step_at(unix), 8)
    }

    #[test]
    fn rfc6238_appendix_b_vectors() {
        assert_eq!(code8(59), "94287082");
        assert_eq!(code8(1_111_111_109), "07081804");
        assert_eq!(code8(1_111_111_111), "14050471");
        assert_eq!(code8(1_234_567_890), "89005924");
        assert_eq!(code8(2_000_000_000), "69279037");
        assert_eq!(code8(20_000_000_000), "65353130");
    }

    #[test]
    fn verify_accepts_current_and_neighbour_steps() {
        let secret = generate_secret();
        let t = 1_700_000_000;
        let now = code_now(&secret, t);
        assert!(verify(&secret, &now, t, None).is_some());
        // Ein Schritt in der Vergangenheit wird noch akzeptiert (SKEW).
        let prev = code_for_step(&secret, step_at(t) - 1, DIGITS);
        assert!(verify(&secret, &prev, t, None).is_some());
        // Zwei Schritte daneben nicht mehr.
        let far = code_for_step(&secret, step_at(t) + 2, DIGITS);
        assert!(verify(&secret, &far, t, None).is_none());
    }

    #[test]
    fn verify_rejects_replay_via_min_step() {
        let secret = generate_secret();
        let t = 1_700_000_000;
        let step = step_at(t);
        let code = code_now(&secret, t);
        // Frisch akzeptiert, Schritt zurückgegeben.
        assert_eq!(verify(&secret, &code, t, None), Some(step));
        // Mit min_step = step (bereits benutzt) nicht mehr.
        assert_eq!(verify(&secret, &code, t, Some(step)), None);
    }

    #[test]
    fn verify_rejects_malformed_input() {
        let secret = generate_secret();
        assert!(verify(&secret, "12345", 1_700_000_000, None).is_none());
        assert!(verify(&secret, "abcdef", 1_700_000_000, None).is_none());
        assert!(verify(&secret, "1234567", 1_700_000_000, None).is_none());
    }

    #[test]
    fn otpauth_uri_contains_secret_and_params() {
        let secret = generate_secret();
        let uri = otpauth_uri("Lineal", "owner@acme.de", &secret);
        assert!(uri.starts_with("otpauth://totp/Lineal:owner%40acme.de?"));
        assert!(uri.contains(&format!("secret={}", base32_encode(&secret))));
        assert!(uri.contains("issuer=Lineal"));
        assert!(uri.contains("digits=6"));
    }

    #[test]
    fn generated_secrets_differ() {
        assert_ne!(generate_secret(), generate_secret());
    }

    #[test]
    fn base32_roundtrips() {
        let secret = generate_secret();
        let decoded = base32_decode(&base32_encode(&secret)).unwrap();
        assert_eq!(decoded, secret);
    }
}
