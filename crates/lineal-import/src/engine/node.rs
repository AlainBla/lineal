//! Lesezugriff auf normalisierte Quellobjekte und Bereinigung von Texten/Enums.
//!
//! Alles aus der Quelle ist nicht vertrauenswürdig: Typen werden geprüft, Längen begrenzt,
//! Farben/Icons gegen feste Muster geprüft, ungültige Enum-Werte durch Standardwerte ersetzt
//! (der Aufrufer meldet das im Bericht).

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Sicht auf ein Quellobjekt.
#[derive(Clone, Copy)]
pub struct N<'a>(pub &'a Value);

impl<'a> N<'a> {
    pub fn str(&self, key: &str) -> Option<&'a str> {
        self.0.get(key)?.as_str()
    }

    /// Nicht-leerer, getrimmter Text.
    pub fn text(&self, key: &str) -> Option<&'a str> {
        self.str(key).map(str::trim).filter(|s| !s.is_empty())
    }

    /// `{ "id": "…" }` -> ID.
    pub fn ref_id(&self, key: &str) -> Option<&'a str> {
        self.0.get(key)?.get("id")?.as_str().filter(|s| !s.is_empty())
    }

    pub fn ts(&self, key: &str) -> Option<DateTime<Utc>> {
        self.str(key)?.parse::<DateTime<Utc>>().ok()
    }

    pub fn date(&self, key: &str) -> Option<String> {
        let s = self.str(key)?;
        let head = s.get(..10)?;
        NaiveDate::parse_from_str(head, "%Y-%m-%d").ok().map(|d| d.to_string())
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        let v = self.0.get(key)?;
        v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))
    }

    pub fn float(&self, key: &str) -> Option<f64> {
        self.0.get(key)?.as_f64().filter(|f| f.is_finite())
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.0.get(key)?.as_bool()
    }

    /// IDs aus einer Liste `[ { "id": … } ]` (begrenzt auf `max`).
    pub fn ref_ids(&self, key: &str, max: usize) -> Vec<&'a str> {
        self.0
            .get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.get("id")?.as_str())
                    .take(max)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn array(&self, key: &str) -> &'a [Value] {
        self.0
            .get(key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `id` des Objekts (Pflicht).
    pub fn id(&self) -> Option<&'a str> {
        self.text("id")
    }

    /// Zahlenliste (z. B. Scope-Historien), begrenzt.
    pub fn floats(&self, key: &str, max: usize) -> Vec<f64> {
        self.0
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).filter(|f| f.is_finite()).take(max).collect())
            .unwrap_or_default()
    }
}

/// SHA-256 über das kanonische JSON (Schlüssel sortiert) – die `checksum` im Mapping.
pub fn checksum_of(v: &Value) -> String {
    let mut out = String::new();
    canonical(v, &mut out);
    hex::encode(Sha256::digest(out.as_bytes()))
}

fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for k in keys {
                out.push_str(&json!(k).to_string());
                out.push(':');
                canonical(&m[k], out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for x in a {
                canonical(x, out);
                out.push(',');
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Steuerzeichen entfernen (außer Zeilenumbruch/Tab bei `multiline`), begrenzen.
pub fn clean_text(s: &str, max_chars: usize, multiline: bool) -> String {
    s.chars()
        .filter(|c| !c.is_control() || (multiline && matches!(c, '\n' | '\t')))
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Einzeiliger Titel/Name: nicht leer (sonst `fallback`), höchstens `max` Zeichen.
pub fn clean_title(s: Option<&str>, max: usize, fallback: &str) -> String {
    let t = clean_text(s.unwrap_or(""), max, false);
    if t.is_empty() { fallback.to_owned() } else { t }
}

/// Farbe `^#[0-9A-Fa-f]{6}$`, sonst `default`.
pub fn color_or(c: Option<&str>, default: &str) -> String {
    match c {
        Some(c) if c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit()) => {
            c.to_owned()
        }
        _ => default.to_owned(),
    }
}

/// Icon `^[A-Za-z0-9_:-]{1,64}$`, sonst `None`.
pub fn icon_or_none(i: Option<&str>) -> Option<String> {
    let i = i?;
    let ok = (1..=64).contains(&i.len()) && i.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-'));
    ok.then(|| i.to_owned())
}

/// Wert aus einer festen Menge, sonst `None`.
pub fn one_of<'a>(v: Option<&str>, allowed: &'a [&'a str]) -> Option<&'a str> {
    let v = v?;
    allowed.iter().copied().find(|a| *a == v)
}

/// JSON-Objekt aus Paaren; `Value::Null` bleibt als NULL erhalten.
pub fn row(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// Zeitpunkt als JSON-String (oder null).
pub fn jts(t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| json!(t.to_rfc3339()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_order_independent() {
        let a = json!({"a":1,"b":{"x":[1,2],"y":null}});
        let b = json!({"b":{"y":null,"x":[1,2]},"a":1});
        assert_eq!(checksum_of(&a), checksum_of(&b));
        assert_ne!(checksum_of(&a), checksum_of(&json!({"a":2,"b":{"x":[1,2],"y":null}})));
    }

    #[test]
    fn validators() {
        assert_eq!(color_or(Some("#A1b2C3"), "#000000"), "#A1b2C3");
        assert_eq!(color_or(Some("red"), "#000000"), "#000000");
        assert_eq!(color_or(Some("#12345"), "#000000"), "#000000");
        assert_eq!(icon_or_none(Some("Rocket")).as_deref(), Some("Rocket"));
        assert_eq!(icon_or_none(Some("<script>")), None);
        assert_eq!(clean_title(Some("  a\u{0}b  "), 10, "x"), "ab");
        assert_eq!(clean_title(Some("   "), 10, "untitled"), "untitled");
        assert_eq!(one_of(Some("started"), &["started", "backlog"]), Some("started"));
        assert_eq!(one_of(Some("bogus"), &["started"]), None);
    }
}
