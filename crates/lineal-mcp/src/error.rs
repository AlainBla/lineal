//! `error` (WP04): Abbildung `AppError` → `isError`-Tool-Ergebnis bzw. JSON-RPC-Fehler.
//!
//! Fachliche und Validierungsfehler sind **Tool-Ergebnisse** mit `isError: true` und dem
//! JSON-Fehlermodell aus docs/00 §7 als Text (docs/07 §1.2). JSON-RPC-Fehlerobjekte gibt es nur
//! für Protokollprobleme (`-32700`, `-32600`, `-32601`, `-32602`).

use lineal_app::AppError;
use serde_json::{Value, json};

/// Parse error: Body ist kein gültiges JSON.
pub const PARSE_ERROR: i64 = -32700;
/// Invalid Request: kein gültiges JSON-RPC-Objekt.
pub const INVALID_REQUEST: i64 = -32600;
/// Method not found (auch unbekannter Tool-Name).
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Invalid params (z. B. `arguments` kein Objekt).
pub const INVALID_PARAMS: i64 = -32602;

/// Fehlermodell nach docs/00 §7: `{ error, message, status, requestId, details }`.
pub fn error_body(err: &AppError, request_id: &str) -> Value {
    json!({
        "error": err.kind.code(),
        "message": err.message,
        "status": err.kind.http_status(),
        "requestId": request_id,
        "details": err.details,
    })
}

/// Tool-Ergebnis für einen Fehler: ein `text`-Content mit dem Fehlermodell und `isError: true`.
pub fn error_result(err: &AppError, request_id: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": error_body(err, request_id).to_string() }],
        "isError": true,
    })
}

/// JSON-RPC-Fehlerantwort (nur für Protokollprobleme).
pub fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::ErrorKind;

    #[test]
    fn error_result_carries_the_error_model_as_text() {
        let err = AppError::not_found("Could not find referenced Issue.")
            .with_details(json!({ "reference": "ENG-999999" }));
        let res = error_result(&err, "req_1");
        assert_eq!(res["isError"], true);
        assert_eq!(res["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(res["content"][0]["type"], "text");
        let body: Value = serde_json::from_str(res["content"][0]["text"].as_str().unwrap_or(""))
            .expect("text is JSON");
        assert_eq!(body["error"], "not_found");
        assert_eq!(body["status"], 404);
        assert_eq!(body["requestId"], "req_1");
        assert_eq!(body["details"]["reference"], "ENG-999999");
    }

    #[test]
    fn every_error_kind_maps_to_code_and_status() {
        for kind in ErrorKind::ALL {
            let body = error_body(&AppError::new(kind, "m"), "r");
            assert_eq!(body["error"], kind.code());
            assert_eq!(body["status"], kind.http_status());
        }
    }

    #[test]
    fn rpc_error_shape() {
        let e = rpc_error(json!(3), METHOD_NOT_FOUND, "Unknown tool");
        assert_eq!(e["error"]["code"], -32601);
        assert_eq!(e["id"], 3);
    }
}
