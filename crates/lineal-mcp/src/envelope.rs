//! `envelope` (WP04): Antwortumschläge für Handler (docs/07 §1.2, §2.2, §3.0).
//!
//! Handler liefern `AppResult<Value>`; der Transport verpackt den Wert mit [`tool_result`].
//! Dieses Modul stellt die Bausteine bereit, damit alle Handler dieselben Formen erzeugen:
//! Paginierungshülle mit opakem, filtergebundenem Cursor, eingebettete Referenzobjekte
//! (`TeamRef`, `UserRef`, …), `Priority`, `{success,id}` und den Hook für Nicht-Text-Content.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::Priority;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Display;

/// Schlüssel, mit dem ein Handler eigenen `content` liefern kann (z. B. `extract_images`:
/// `type: "image"`). Wert: Array fertiger MCP-Content-Elemente. Der Transport verwendet es
/// unverändert als `content` und entfernt den Schlüssel; ohne ihn gilt die Standardform.
pub const CONTENT_KEY: &str = "_mcpContent";

/// Standard-Limit der Listen-Tools.
pub const DEFAULT_LIMIT: usize = 50;
/// Maximales Limit der meisten Listen-Tools (`list_projects`: 50).
pub const MAX_LIMIT: usize = 250;

/// Erfolgs-Ergebnis: genau ein `text`-Content mit kompaktem JSON, `isError: false`.
/// Enthält `value` den Schlüssel [`CONTENT_KEY`] (Array), wird dieser als `content` verwendet.
pub fn tool_result(value: Value) -> Value {
    if let Value::Object(mut map) = value {
        if let Some(content @ Value::Array(_)) = map.remove(CONTENT_KEY) {
            return json!({ "content": content, "isError": false });
        }
        return text_result(&Value::Object(map));
    }
    text_result(&value)
}

fn text_result(value: &Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "isError": false,
    })
}

/// `{ "success": true, "id": "<uuid>" }` für `delete_*`/`retire_*`/`restore_*`.
pub fn success(id: impl Display) -> Value {
    json!({ "success": true, "id": id.to_string() })
}

// ---------------------------------------------------------------------------------------------
// Paginierung (docs/07 §2.2)
// ---------------------------------------------------------------------------------------------

/// Liest und prüft `limit` (Default `default`, 1..=`max`); sonst `invalid_request`.
pub fn parse_limit(args: &Value, default: usize, max: usize) -> AppResult<usize> {
    let Some(raw) = args.get("limit").filter(|v| !v.is_null()) else {
        return Ok(default);
    };
    let n = raw.as_f64().unwrap_or(0.0);
    if n.fract() != 0.0 || n < 1.0 || n > max as f64 {
        return Err(AppError::invalid_request(format!(
            "limit must be an integer between 1 and {max}"
        ))
        .with_details(json!({ "parameter": "limit", "max": max })));
    }
    Ok(n as usize)
}

/// Inhalt eines dekodierten Cursors: Sortschlüssel `k` und Tiebreaker `i` (UUID).
#[derive(Debug, Clone, PartialEq)]
pub struct CursorPos {
    pub key: Value,
    pub id: String,
}

/// Kurzer Hash der Filterparameter: SHA-256 über die kanonische (Schlüssel sortierte) Form von
/// `args` ohne `cursor`, `limit`, `fields` – gleiche Filter ergeben denselben Hash.
pub fn query_fingerprint(args: &Value) -> String {
    fn canon(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<_> = m.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for k in keys {
                    out.insert(k.clone(), canon(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
            other => other.clone(),
        }
    }
    let mut stripped = canon(args);
    if let Value::Object(m) = &mut stripped {
        for k in ["cursor", "limit", "fields"] {
            m.shift_remove(k);
        }
    }
    let digest = Sha256::digest(stripped.to_string().as_bytes());
    hex::encode(&digest[..6])
}

/// Erzeugt einen opaken Cursor `base64url({"k": key, "i": id, "h": fingerprint})`.
pub fn encode_cursor(key: &Value, id: impl Display, fingerprint: &str) -> String {
    let raw = json!({ "k": key, "i": id.to_string(), "h": fingerprint });
    URL_SAFE_NO_PAD.encode(raw.to_string())
}

/// Dekodiert einen Cursor. Fremde Filter → `invalid_request` („Cursor does not match the
/// query“), kaputter Cursor → `invalid_request` („Invalid cursor“).
pub fn decode_cursor(cursor: &str, fingerprint: &str) -> AppResult<CursorPos> {
    let invalid = || AppError::invalid_request("Invalid cursor");
    let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let id = v.get("i").and_then(Value::as_str).ok_or_else(invalid)?;
    let hash = v.get("h").and_then(Value::as_str).ok_or_else(invalid)?;
    if hash != fingerprint {
        return Err(AppError::invalid_request("Cursor does not match the query")
            .with_details(json!({ "parameter": "cursor" })));
    }
    Ok(CursorPos {
        key: v.get("k").cloned().unwrap_or(Value::Null),
        id: id.to_owned(),
    })
}

/// Schneidet `limit + 1` geladene Zeilen auf `limit` zurück und bildet den Cursor aus der
/// letzten behaltenen Zeile (`None` bei letzter Seite). Handler laden also `limit + 1` Zeilen.
pub fn paginate<T>(
    mut rows: Vec<T>,
    limit: usize,
    cursor_of: impl Fn(&T) -> String,
) -> (Vec<T>, Option<String>) {
    if rows.len() > limit {
        rows.truncate(limit);
        let cursor = rows.last().map(cursor_of);
        (rows, cursor)
    } else {
        (rows, None)
    }
}

/// Paginierungshülle `{ "<key>": [...], "hasNextPage": bool, "cursor"?: "..." }`; `cursor`
/// fehlt bei `hasNextPage: false`. Auch für Listen ohne `cursor`-Parameter (immer
/// `hasNextPage: false`, siehe docs/07 §2.2) zu verwenden: `page(key, items, None)`.
pub fn page(key: &str, items: Vec<Value>, next_cursor: Option<String>) -> Value {
    let mut map = Map::new();
    map.insert(key.to_owned(), Value::Array(items));
    map.insert("hasNextPage".into(), Value::Bool(next_cursor.is_some()));
    if let Some(c) = next_cursor {
        map.insert("cursor".into(), Value::String(c));
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------------------------
// Eingebettete Referenzobjekte (docs/07 §3.0)
// ---------------------------------------------------------------------------------------------

/// `TeamRef`: `{ id, key, name }`.
pub fn team_ref(id: impl Display, key: &str, name: &str) -> Value {
    json!({ "id": id.to_string(), "key": key, "name": name })
}

/// `UserRef`: `{ id, name }`.
pub fn user_ref(id: impl Display, name: &str) -> Value {
    json!({ "id": id.to_string(), "name": name })
}

/// `UserRef` für Projekt-Mitglieder: zusätzlich `email`.
pub fn member_ref(id: impl Display, name: &str, email: &str) -> Value {
    json!({ "id": id.to_string(), "name": name, "email": email })
}

/// `ProjectRef`: `id` ist die UUID, `identifier` die lesbare Kennung.
pub fn project_ref(id: impl Display, identifier: &str, name: &str) -> Value {
    json!({ "id": id.to_string(), "identifier": identifier, "name": name })
}

/// `InitiativeRef`: wie [`project_ref`].
pub fn initiative_ref(id: impl Display, identifier: &str, name: &str) -> Value {
    json!({ "id": id.to_string(), "identifier": identifier, "name": name })
}

/// `StatusRef` (Workflow-, Projekt- und Initiative-Status): `{ id, name, type }`.
pub fn status_ref(id: impl Display, name: &str, kind: &str) -> Value {
    json!({ "id": id.to_string(), "name": name, "type": kind })
}

/// `LabelRef`: `{ id, name }`.
pub fn label_ref(id: impl Display, name: &str) -> Value {
    json!({ "id": id.to_string(), "name": name })
}

/// `MilestoneRef`: `{ id, name }`.
pub fn milestone_ref(id: impl Display, name: &str) -> Value {
    json!({ "id": id.to_string(), "name": name })
}

/// `Priority`: `{ "value": 2, "name": "High" }`; ungültiger Wert → `invalid_request`.
pub fn priority(value: u8) -> AppResult<Value> {
    let p = Priority::try_from(value)?;
    Ok(json!({ "value": p.value(), "name": p.name() }))
}

/// Wert oder `null` (Regel „leer = null“, docs/07 §2.3); leerer String → `null` (§4 Nr. 12).
pub fn opt_text(text: Option<&str>) -> Value {
    match text {
        Some(t) if !t.is_empty() => Value::String(t.to_owned()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_wraps_compact_json_text() {
        let res = tool_result(json!({ "id": "ENG-1" }));
        assert_eq!(res["isError"], false);
        assert_eq!(res["content"][0]["type"], "text");
        assert_eq!(res["content"][0]["text"], r#"{"id":"ENG-1"}"#);
    }

    #[test]
    fn tool_result_honours_custom_content() {
        let res = tool_result(json!({
            CONTENT_KEY: [{ "type": "image", "data": "AAAA", "mimeType": "image/png" }]
        }));
        assert_eq!(res["content"][0]["type"], "image");
        assert_eq!(res["isError"], false);
    }

    #[test]
    fn page_omits_cursor_on_last_page() {
        let last = page("issues", vec![json!({"id": "a"})], None);
        assert_eq!(
            last,
            json!({ "issues": [{"id": "a"}], "hasNextPage": false })
        );
        let more = page("issues", vec![], Some("abc".into()));
        assert_eq!(more["hasNextPage"], true);
        assert_eq!(more["cursor"], "abc");
    }

    #[test]
    fn paginate_uses_limit_plus_one() {
        let (rows, cursor) = paginate(vec![1, 2, 3], 2, |r| r.to_string());
        assert_eq!(rows, [1, 2]);
        assert_eq!(cursor.as_deref(), Some("2"));
        let (rows, cursor) = paginate(vec![1, 2], 2, |r| r.to_string());
        assert_eq!(rows.len(), 2);
        assert!(cursor.is_none());
    }

    #[test]
    fn cursor_roundtrip_and_filter_binding() {
        let args = json!({ "team": "ENG", "limit": 10, "cursor": "x" });
        let fp = query_fingerprint(&args);
        // limit/cursor/fields zählen nicht zum Fingerprint, Schlüsselreihenfolge auch nicht.
        assert_eq!(
            fp,
            query_fingerprint(&json!({ "limit": 50, "team": "ENG" }))
        );
        assert_ne!(fp, query_fingerprint(&json!({ "team": "WEB" })));

        let c = encode_cursor(&json!("2026-10-10T07:15:00.000Z"), "9f1c", &fp);
        assert!(!c.contains('=') && !c.contains('+') && !c.contains('/'));
        let pos = decode_cursor(&c, &fp).expect("valid");
        assert_eq!(pos.id, "9f1c");
        assert_eq!(pos.key, json!("2026-10-10T07:15:00.000Z"));

        let other = query_fingerprint(&json!({ "team": "WEB" }));
        let err = decode_cursor(&c, &other).expect_err("other filter");
        assert_eq!(err.message, "Cursor does not match the query");
        assert!(decode_cursor("not-a-cursor!", &fp).is_err());
    }

    #[test]
    fn parse_limit_rules() {
        assert_eq!(parse_limit(&json!({}), 50, 250).ok(), Some(50));
        assert_eq!(parse_limit(&json!({"limit": 250}), 50, 250).ok(), Some(250));
        assert!(parse_limit(&json!({"limit": 0}), 50, 250).is_err());
        assert!(parse_limit(&json!({"limit": 251}), 50, 250).is_err());
        assert!(parse_limit(&json!({"limit": 1.5}), 50, 250).is_err());
    }

    #[test]
    fn reference_shapes() {
        assert_eq!(
            team_ref("u1", "ENG", "Engineering"),
            json!({"id": "u1", "key": "ENG", "name": "Engineering"})
        );
        assert_eq!(
            status_ref("u2", "In Progress", "started"),
            json!({"id": "u2", "name": "In Progress", "type": "started"})
        );
        assert_eq!(
            project_ref("u3", "P-ENG-7", "Checkout v2")["identifier"],
            "P-ENG-7"
        );
        assert_eq!(priority(2).ok(), Some(json!({"value": 2, "name": "High"})));
        assert!(priority(9).is_err());
        assert_eq!(success("u4"), json!({"success": true, "id": "u4"}));
        assert_eq!(opt_text(Some("")), Value::Null);
    }
}
