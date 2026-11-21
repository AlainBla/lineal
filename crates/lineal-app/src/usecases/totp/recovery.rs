//! Recovery-Codes: Einmal-Ersatzcodes für den Verlust des Authenticators.
//!
//! Erzeugung aus dem CSPRNG mit einem Alphabet ohne Zeichenverwechsler
//! (kein 0/O/1/I/L). 8 Zeichen aus 32 ≈ 40 Bit Entropie — genug, dass ein
//! **keyed HMAC-SHA256** zum Speichern reicht (kein Argon2 im Login-Pfad).
//! Vergleich konstantzeitig. Eingaben werden vor Hash/Vergleich normalisiert
//! (Großschreibung, Bindestriche/Leerzeichen entfernt).

use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Alphabet ohne 0/O/1/I/L — leicht abzutippen.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Zeichen je Code (ohne Bindestrich).
const CODE_CHARS: usize = 8;
/// Anzahl der Codes pro Satz.
pub const CODE_COUNT: usize = 10;

/// Erzeugt einen frischen Satz Recovery-Codes im Format `XXXX-XXXX`.
pub fn generate_codes() -> Vec<String> {
    let mut rng = rand::rngs::OsRng;
    (0..CODE_COUNT)
        .map(|_| {
            let chars: String = (0..CODE_CHARS)
                .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                .collect();
            format!("{}-{}", &chars[..4], &chars[4..])
        })
        .collect()
}

/// Normalisiert eine Code-Eingabe: Großschreibung, nur Alphabet-Zeichen.
pub fn normalize(code: &str) -> String {
    code.chars()
        .filter_map(|c| {
            let u = c.to_ascii_uppercase();
            ALPHABET.contains(&(u as u8)).then_some(u)
        })
        .collect()
}

/// HMAC-SHA256 des normalisierten Codes unter `key`.
pub fn hash_code(key: &[u8], code: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC akzeptiert jede Schlüssellänge");
    mac.update(normalize(code).as_bytes());
    mac.finalize().into_bytes().into()
}

/// Konstantzeitiger Vergleich einer Eingabe gegen einen gespeicherten Hash.
pub fn verify_code(key: &[u8], code: &str, stored_hash: &[u8]) -> bool {
    let computed = hash_code(key, code);
    computed.ct_eq(stored_hash).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_ten_unique_formatted_codes() {
        let codes = generate_codes();
        assert_eq!(codes.len(), CODE_COUNT);
        for c in &codes {
            assert_eq!(c.len(), 9, "XXXX-XXXX");
            assert_eq!(c.as_bytes()[4], b'-');
        }
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), CODE_COUNT, "Codes sind eindeutig");
    }

    #[test]
    fn hash_verifies_and_is_normalization_insensitive() {
        let key = b"test-key-123456789012345678901234";
        let code = "4K2P-9QX7";
        let h = hash_code(key, code);
        assert!(verify_code(key, code, &h));
        // Kleinschreibung und fehlender Bindestrich müssen trotzdem passen.
        assert!(verify_code(key, "4k2p9qx7", &h));
        assert!(verify_code(key, " 4K2P - 9QX7 ", &h));
    }

    #[test]
    fn wrong_code_and_wrong_key_fail() {
        let key = b"test-key-123456789012345678901234";
        let h = hash_code(key, "4K2P-9QX7");
        assert!(!verify_code(key, "4K2P-9QX8", &h));
        assert!(!verify_code(
            b"other-key-00000000000000000000000",
            "4K2P-9QX7",
            &h
        ));
    }
}
