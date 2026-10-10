//! `fields` (WP04): `fields`-Auswahl (docs/07 §2.3) und Kürzung langer Texte (§2.9).

use serde_json::{Map, Value};

/// Maximale Länge (Unicode-Codepoints) von `description`/`content` in Listenantworten.
pub const TRUNCATE_LIMIT: usize = 500;

/// Liest `args.fields` (Array von Strings, Duplikate entfernt, Reihenfolge erhalten).
/// Leer = Default-Antwort des Tools.
pub fn parse_fields(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(items) = args.get("fields").and_then(Value::as_array) {
        for f in items.iter().filter_map(Value::as_str) {
            if !out.iter().any(|o| o == f) {
                out.push(f.to_owned());
            }
        }
    }
    out
}

/// Wendet die `fields`-Regeln auf ein vollständig gebautes Objekt an.
///
/// - `requested` leer → `full` unverändert (Default-Antwort, alle Default-Schlüssel vorhanden).
/// - sonst: genau `id` (zuerst) plus die angefragten Schlüssel in angefragter Reihenfolge;
///   fehlende Werte → `null`, Felder aus `list_fields` → `[]`.
pub fn select_fields(
    full: &Map<String, Value>,
    requested: &[String],
    list_fields: &[&str],
) -> Value {
    if requested.is_empty() {
        return Value::Object(full.clone());
    }
    let mut out = Map::new();
    out.insert("id".into(), full.get("id").cloned().unwrap_or(Value::Null));
    for key in requested.iter().filter(|k| k.as_str() != "id") {
        let empty_list = list_fields.contains(&key.as_str());
        let value = match full.get(key) {
            Some(Value::Null) | None if empty_list => Value::Array(vec![]),
            Some(v) => v.clone(),
            None => Value::Null,
        };
        out.insert(key.clone(), value);
    }
    Value::Object(out)
}

/// Kürzt `text` auf [`TRUNCATE_LIMIT`] Codepoints (am letzten Whitespace davor, wenn möglich)
/// und hängt ``… (truncated, use `<get_tool>` for full <noun>)`` an. Kürzere Texte bleiben
/// unverändert.
pub fn truncate_text(text: &str, get_tool: &str, noun: &str) -> String {
    if text.chars().count() <= TRUNCATE_LIMIT {
        return text.to_owned();
    }
    // Byte-Offset der Position 500 (Codepoint-Grenze, daher nie mitten in einem Zeichen).
    let cut = text
        .char_indices()
        .nth(TRUNCATE_LIMIT)
        .map_or(text.len(), |(i, _)| i);
    let head = &text[..cut];
    let head = match head.rfind(char::is_whitespace) {
        Some(i) if i > 0 => &head[..i],
        _ => head,
    };
    format!(
        "{} … (truncated, use `{get_tool}` for full {noun})",
        head.trim_end()
    )
}

/// Kürzung einer `description` (Issue/Projekt/Initiative).
pub fn truncate_description(text: &str, get_tool: &str) -> String {
    truncate_text(text, get_tool, "description")
}

/// Kürzung eines Dokument-`content`.
pub fn truncate_content(text: &str, get_tool: &str) -> String {
    truncate_text(text, get_tool, "content")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn short_text_is_untouched() {
        let s = "a".repeat(500);
        assert_eq!(truncate_description(&s, "get_issue"), s);
    }

    #[test]
    fn long_text_is_cut_with_suffix() {
        let s = "a".repeat(501);
        let out = truncate_description(&s, "get_issue");
        assert_eq!(
            out,
            format!(
                "{} … (truncated, use `get_issue` for full description)",
                "a".repeat(500)
            )
        );
    }

    #[test]
    fn cut_prefers_last_whitespace() {
        let s = format!("{} {}", "a".repeat(495), "b".repeat(50));
        let out = truncate_description(&s, "get_project");
        assert!(out.starts_with(&"a".repeat(495)));
        assert!(!out.contains('b'));
        assert!(out.ends_with("(truncated, use `get_project` for full description)"));
    }

    #[test]
    fn emoji_at_position_500_is_not_split() {
        let s = format!("{}{}", "a".repeat(499), "😀😀😀");
        let out = truncate_description(&s, "get_issue");
        assert!(out.starts_with(&format!("{}😀 …", "a".repeat(499))));
        let content = truncate_content(&s, "get_document");
        assert!(content.ends_with("for full content)"));
    }

    #[test]
    fn select_fields_id_first_nulls_and_lists() {
        let full = obj(json!({"title": "T", "id": "ENG-1", "labels": null, "state": "Todo"}));
        let out = select_fields(
            &full,
            &[
                "state".into(),
                "labels".into(),
                "assignee".into(),
                "title".into(),
            ],
            &["labels"],
        );
        let keys: Vec<String> = out
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(keys, ["id", "state", "labels", "assignee", "title"]);
        assert_eq!(out["labels"], json!([]));
        assert_eq!(out["assignee"], Value::Null);
        assert_eq!(out["state"], "Todo");
    }

    #[test]
    fn empty_fields_returns_default_object() {
        let full = obj(json!({"id": "ENG-1", "title": "T"}));
        assert_eq!(select_fields(&full, &[], &[]), Value::Object(full.clone()));
        assert!(parse_fields(&json!({})).is_empty());
        assert_eq!(
            parse_fields(&json!({"fields": ["a", "b", "a"]})),
            ["a", "b"]
        );
    }
}
