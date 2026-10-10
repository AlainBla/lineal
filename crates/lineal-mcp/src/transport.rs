//! `transport` (WP04): JSON-RPC 2.0 über Streamable HTTP (`/mcp`), docs/07 §1.2.
//!
//! Eigene, schlanke Umsetzung (zustandslos): `initialize` (mit Aushandlung der Protokollversion),
//! `ping`, `tools/list` (nach Scopes gefiltert), `tools/call` (Scope → Schema-Validierung →
//! Dispatch → `isError`-Abbildung). Antwort als `application/json`, oder als einzelnes
//! SSE-Event, wenn der Client nur `text/event-stream` akzeptiert. `GET /mcp` → 405 (kein
//! Server→Client-Stream), `DELETE /mcp` → 204 (Session ist zustandslos).

use crate::auth::{self, authenticate, ensure_scope, failure_response, scope_allows};
use crate::dispatch;
use crate::envelope::tool_result;
use crate::error::{
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, error_result, rpc_error,
};
use crate::validate::{catalog, validate_args};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lineal_app::{App, RequestCtx};
use serde_json::{Value, json};
use std::sync::Arc;

/// Unterstützte MCP-Protokollversionen, neueste zuerst.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Hinweis aus `initialize.instructions` (docs/07 §1.2).
pub const INSTRUCTIONS: &str = "When passing string values to tools, send the content directly without escape sequences. For example, use real newlines in markdown content rather than literal backslash-n (\\n) characters.";

const SESSION_HEADER: HeaderName = HeaderName::from_static("mcp-session-id");
const PROTOCOL_HEADER: &str = "mcp-protocol-version";

/// Höchste gemeinsame Version: die angefragte, wenn unterstützt, sonst die neueste des Servers.
pub fn negotiate_protocol_version(requested: &str) -> &'static str {
    SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .copied()
        .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0])
}

type RpcResult = Result<Value, (i64, String)>;

fn params_err(msg: impl Into<String>) -> (i64, String) {
    (INVALID_PARAMS, msg.into())
}

fn handle_initialize(params: &Value) -> RpcResult {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| params_err("initialize requires params.protocolVersion"))?;
    Ok(json!({
        "protocolVersion": negotiate_protocol_version(requested),
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "lineal", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    }))
}

fn handle_tools_list(ctx: &RequestCtx) -> RpcResult {
    let tools: Vec<Value> = catalog()
        .tools()
        .iter()
        .filter(|t| scope_allows(ctx.scopes, auth::required_scope(t.name)))
        .map(|t| t.to_list_entry())
        .collect();
    Ok(json!({ "tools": tools }))
}

async fn handle_tools_call(app: &App, ctx: &RequestCtx, params: &Value) -> RpcResult {
    let obj = params
        .as_object()
        .ok_or_else(|| params_err("params must be an object"))?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| params_err("params.name must be a string"))?;
    if dispatch::group_for(name).is_none() {
        return Err((METHOD_NOT_FOUND, format!("Unknown tool: {name}")));
    }
    let args = match obj.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err(params_err("params.arguments must be an object")),
    };
    if let Err(e) = ensure_scope(ctx, name) {
        return Ok(error_result(&e, &ctx.request_id));
    }
    if let Err(e) = validate_args(name, &args) {
        return Ok(error_result(&e, &ctx.request_id));
    }
    match dispatch::dispatch(app, ctx, name, args).await {
        None => Err((METHOD_NOT_FOUND, format!("Unknown tool: {name}"))),
        Some(Ok(value)) => Ok(tool_result(value)),
        Some(Err(e)) => {
            if e.kind == lineal_app::ErrorKind::Internal {
                tracing::error!(tool = name, request_id = %ctx.request_id, error = %e, "tool failed");
            }
            Ok(error_result(&e, &ctx.request_id))
        }
    }
}

/// Verarbeitet eine einzelne JSON-RPC-Nachricht. `None` = keine Antwort (Notification oder
/// Antwort des Clients).
pub async fn handle_message(app: &App, ctx: &RequestCtx, msg: Value) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(rpc_error(Value::Null, INVALID_REQUEST, "Invalid Request"));
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // Antwort eines Clients (result/error) wird quittiert, nicht beantwortet.
        let is_response = obj.contains_key("result") || obj.contains_key("error");
        return (!is_response).then(|| {
            rpc_error(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: missing method",
            )
        });
    };
    let id = match obj.get("id") {
        None => None,
        Some(v @ (Value::String(_) | Value::Number(_))) => Some(v.clone()),
        Some(_) => {
            return Some(rpc_error(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: bad id",
            ));
        }
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ));
    }
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(id) = id else {
        return None; // Notification (z. B. notifications/initialized)
    };
    let result = match method {
        "initialize" => handle_initialize(&params),
        "ping" => Ok(json!({})),
        "tools/list" => handle_tools_list(ctx),
        "tools/call" => handle_tools_call(app, ctx, &params).await,
        other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, message),
    })
}

fn wants_sse_only(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/event-stream") && !a.contains("application/json"))
}

fn json_response(status: StatusCode, body: &Value, sse: bool) -> Response {
    if sse {
        let payload = format!("event: message\ndata: {body}\n\n");
        let mut r = (status, payload).into_response();
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        r.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        r
    } else {
        (status, axum::Json(body)).into_response()
    }
}

/// `POST /mcp`.
pub async fn post_mcp(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let ctx = match authenticate(&app, &headers).await {
        Ok(ctx) => ctx,
        Err(failure) => return failure_response(&app, failure),
    };
    if let Some(v) = headers.get(PROTOCOL_HEADER).and_then(|v| v.to_str().ok())
        && !SUPPORTED_PROTOCOL_VERSIONS.contains(&v)
    {
        let err = rpc_error(
            Value::Null,
            INVALID_REQUEST,
            format!("Unsupported MCP-Protocol-Version: {v}"),
        );
        return json_response(StatusCode::BAD_REQUEST, &err, false);
    }
    let sse = wants_sse_only(&headers);
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let err = rpc_error(Value::Null, PARSE_ERROR, format!("Parse error: {e}"));
            return json_response(StatusCode::BAD_REQUEST, &err, sse);
        }
    };

    let is_initialize = |m: &Value| m.get("method").and_then(Value::as_str) == Some("initialize");
    let (response, initialize) = match parsed {
        Value::Array(items) if items.is_empty() => (
            Some(rpc_error(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: empty batch",
            )),
            false,
        ),
        Value::Array(items) => {
            let initialize = items.iter().any(is_initialize);
            let mut out = Vec::new();
            for item in items {
                if let Some(r) = handle_message(&app, &ctx, item).await {
                    out.push(r);
                }
            }
            ((!out.is_empty()).then_some(Value::Array(out)), initialize)
        }
        single => {
            let initialize = is_initialize(&single);
            (handle_message(&app, &ctx, single).await, initialize)
        }
    };

    let Some(body) = response else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut resp = json_response(StatusCode::OK, &body, sse);
    if initialize && let Ok(v) = HeaderValue::from_str(&uuid::Uuid::now_v7().to_string()) {
        resp.headers_mut().insert(SESSION_HEADER, v);
    }
    resp
}

/// `GET /mcp`: kein Server→Client-Stream angeboten (zulässig laut Spezifikation).
pub async fn get_mcp() -> Response {
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    r.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
    r
}

/// `DELETE /mcp`: beendet die (zustandslose) Session.
pub async fn delete_mcp(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    match authenticate(&app, &headers).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => failure_response(&app, failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate_protocol_version("2025-06-18"), "2025-06-18");
        assert_eq!(negotiate_protocol_version("2024-11-05"), "2024-11-05");
        assert_eq!(negotiate_protocol_version("2099-01-01"), "2025-11-25");
        assert_eq!(negotiate_protocol_version("garbage"), "2025-11-25");
    }
}
