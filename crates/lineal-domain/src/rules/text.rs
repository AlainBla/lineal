//! Gemeinsame Eingabevalidierung für Texte (Paket 8, B8.01/B8.02/B8.22): einzeilige Namen,
//! NUL-Filter und Normalisierung. Reine Funktionen ohne I/O.

use crate::error::DomainError;
use unicode_normalization::UnicodeNormalization;

/// Unsichtbare oder umlenkende Zeichen, die in einzeiligen Namen nichts zu suchen haben:
/// Zeilen-/Absatztrenner, Bidi-Steuerzeichen (inkl. Override/Isolate), Zero-Width-Zeichen
/// (ZWJ U+200D bleibt erlaubt: Emoji-Sequenzen), Wortverbinder, BOM, Soft Hyphen.
fn is_forbidden_in_name(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'
                | '\u{200C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// Normalisiert nach Unicode NFC (Vergleichs- und Speicherform von Namen).
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// Eindeutigkeitsschlüssel für Namen (B8.22): NFKC (faltet Vollbreite/Kompatibilitätsformen),
/// kleingeschrieben, getrimmt.
pub fn name_key(s: &str) -> String {
    s.nfkc().collect::<String>().trim().to_lowercase()
}

/// Enthält der Text ein NUL-Zeichen (Postgres lehnt es in `text` ab)?
pub fn contains_nul(s: &str) -> bool {
    s.contains('\0')
}

/// Verbietet NUL in einem Textfeld (`invalid_request` statt Datenbankfehler).
pub fn reject_nul(field: &str, s: &str) -> Result<(), DomainError> {
    if contains_nul(s) {
        Err(DomainError::validation(
            field,
            "must not contain NUL characters",
        ))
    } else {
        Ok(())
    }
}

/// Rekursive Prüfung eines JSON-Werts (Strings und Objektschlüssel) auf NUL.
pub fn json_contains_nul(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::String(s) => contains_nul(s),
        serde_json::Value::Array(a) => a.iter().any(json_contains_nul),
        serde_json::Value::Object(o) => o
            .iter()
            .any(|(k, v)| contains_nul(k) || json_contains_nul(v)),
        _ => false,
    }
}

/// Validiert einen einzeiligen Namen: NFC-normalisiert, getrimmt, 1..=`max_chars` Zeichen
/// (gezählt nach der Normalisierung), ohne Steuerzeichen, Zeilenumbrüche, NUL, Bidi- und
/// unsichtbare Zeichen. Liefert die normalisierte Form.
pub fn validate_single_line_name(
    field: &str,
    raw: &str,
    max_chars: usize,
) -> Result<String, DomainError> {
    let name = nfc(raw.trim());
    if name.is_empty() {
        return Err(DomainError::validation(field, "must not be empty"));
    }
    if name.chars().count() > max_chars {
        return Err(DomainError::validation(
            field,
            format!("must be at most {max_chars} characters"),
        ));
    }
    if name.chars().any(is_forbidden_in_name) {
        return Err(DomainError::validation(
            field,
            "must not contain line breaks, control or invisible formatting characters",
        ));
    }
    Ok(name)
}

/// Entfernt Steuerzeichen und Zeilenumbrüche (für Mail-Betreff und -Header, defensiv):
/// jede Folge wird zu einem Leerzeichen.
pub fn sanitize_header_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        if is_forbidden_in_name(c) {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = c == ' ';
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_line_breaks_controls_bidi_and_invisibles() {
        for bad in [
            "a\nb",
            "a\rb",
            "a\0b",
            "a\tb",
            "a\u{2028}b",
            "a\u{2029}b",
            "a\u{202E}b",
            "a\u{200B}b",
            "a\u{2066}b",
            "a\u{FEFF}b",
        ] {
            assert!(
                validate_single_line_name("name", bad, 80).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn normalizes_to_nfc_and_counts_after_normalization() {
        let decomposed = "cafe\u{301}";
        assert_eq!(
            validate_single_line_name("name", decomposed, 4).unwrap(),
            "caf\u{e9}"
        );
        assert!(validate_single_line_name("name", "  ", 80).is_err());
        assert!(validate_single_line_name("name", &"x".repeat(81), 80).is_err());
        assert!(validate_single_line_name("name", "a\u{200D}b", 80).is_ok());
    }

    #[test]
    fn name_key_folds_unicode_variants() {
        assert_eq!(name_key("Caf\u{e9}"), name_key("cafe\u{301}"));
        assert_eq!(name_key("ＡＢＣ"), "abc");
    }

    #[test]
    fn nul_helpers() {
        assert!(reject_nul("f", "a\0").is_err());
        assert!(reject_nul("f", "ok").is_ok());
        assert!(json_contains_nul(&serde_json::json!({"a": ["x\u{0}"]})));
        assert!(!json_contains_nul(&serde_json::json!({"a": ["x"]})));
    }

    #[test]
    fn header_values_lose_line_breaks() {
        assert_eq!(
            sanitize_header_value("X\nERROR: fake\r\nBcc: a"),
            "X ERROR: fake Bcc: a"
        );
    }
}
