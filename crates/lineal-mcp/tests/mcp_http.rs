//! HTTP-Ende-zu-Ende-Tests des MCP-Adapters gegen den Router (ohne Netzwerk, `FakeStore`).

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use lineal_app::ports::repos::Principal;
use lineal_app::{Actor, Scopes};
use lineal_domain::enums::UserRole;
use lineal_mcp::auth::hash_token;
use lineal_mcp::dispatch::{COMPAT_TOOLS, SNAPSHOT_JSON};
use lineal_testkit::{FakeStore, SeedData, test_app};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use tower::ServiceExt;

const RW: &str = "lineal_api_readwrite";
const RO: &str = "lineal_api_readonly";
const SESSION: &str = "web-session-cookie-value";

fn principal(scopes: Scopes) -> Principal {
    let seed = SeedData::acme();
    Principal {
        kind: lineal_app::ports::repos::CredentialKind::ApiKey,
        org_id: seed.org,
        actor: Actor::User {
            id: seed.alice,
            role: UserRole::Owner,
        },
        scopes,
        team_scope: None,
    }
}

fn app() -> Router {
    let store = FakeStore::new()
        .with_principal(hash_token(RW), principal(Scopes::read_write()))
        .with_principal(hash_token(RO), principal(Scopes::read_only()))
        .with_principal(hash_token(SESSION), {
            let mut p = principal(Scopes::all());
            p.kind = lineal_app::ports::repos::CredentialKind::Session;
            p
        });
    lineal_mcp::router(test_app(Arc::new(store)))
}

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    accept: Option<&str>,
    body: &str,
) -> (StatusCode, Response<Body>) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(a) = accept {
        req = req.header(header::ACCEPT, a);
    }
    let resp = router
        .clone()
        .oneshot(
            req.header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    (resp.status(), resp)
}

async fn body_json(resp: Response<Body>) -> Value {
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

async fn rpc(router: &Router, token: &str, msg: Value) -> Value {
    let (status, resp) = send(
        router,
        Method::POST,
        "/mcp",
        Some(token),
        Some("application/json, text/event-stream"),
        &msg.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body_json(resp).await
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": args } })
}

fn tool_error(resp: &Value) -> Value {
    assert_eq!(resp["result"]["isError"], true, "{resp}");
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    serde_json::from_str(text).expect("error model json")
}

#[tokio::test]
async fn missing_token_is_401_with_www_authenticate() {
    let router = app();
    let (status, resp) = send(&router, Method::POST, "/mcp", None, None, "{}").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = resp
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .expect("WWW-Authenticate")
        .to_str()
        .expect("ascii")
        .to_owned();
    assert_eq!(
        challenge,
        r#"Bearer resource_metadata="http://localhost:8080/.well-known/oauth-protected-resource""#
    );
    let body = body_json(resp).await;
    assert_eq!(body["error"], "unauthenticated");
    assert_eq!(body["status"], 401);
}

#[tokio::test]
async fn session_token_as_bearer_is_rejected() {
    let router = app();
    let msg = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let (status, resp) = send(&router, Method::POST, "/mcp", Some(SESSION), None, msg).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(resp.headers().contains_key(header::WWW_AUTHENTICATE));
    // Gegenprobe: API-Key funktioniert weiterhin.
    let (status, _) = send(&router, Method::POST, "/mcp", Some(RW), None, msg).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn invalid_token_is_401_with_invalid_token_challenge() {
    let router = app();
    let (status, resp) = send(&router, Method::POST, "/mcp", Some("nope"), None, "{}").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = resp
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .expect("WWW-Authenticate")
        .to_str()
        .expect("ascii");
    assert!(
        challenge.contains(r#"error="invalid_token""#),
        "{challenge}"
    );
    assert!(challenge.contains("resource_metadata="), "{challenge}");
}

#[tokio::test]
async fn initialize_negotiates_version_and_sets_session() {
    let router = app();
    let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": { "name": "example-agent", "version": "1.0.0" } } });
    let (status, resp) = send(
        &router,
        Method::POST,
        "/mcp",
        Some(RW),
        None,
        &msg.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.headers().contains_key("mcp-session-id"));
    let body = body_json(resp).await;
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        body["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(body["result"]["serverInfo"]["name"], "lineal");
    assert!(body["result"]["instructions"].is_string());

    // Unbekannte Zukunftsversion: Server antwortet mit seiner neuesten.
    let msg = json!({ "jsonrpc": "2.0", "id": "x", "method": "initialize",
        "params": { "protocolVersion": "2099-01-01" } });
    let body = rpc(&router, RW, msg).await;
    assert_eq!(body["id"], "x");
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");

    let missing = json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {} });
    assert_eq!(rpc(&router, RW, missing).await["error"]["code"], -32602);
}

#[tokio::test]
async fn tools_list_matches_snapshot_exactly() {
    let router = app();
    let body = rpc(
        &router,
        RW,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .await;
    let tools = body["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 56);

    let snapshot: Value = serde_json::from_str(SNAPSHOT_JSON).expect("snapshot");
    let compat: BTreeSet<&str> = COMPAT_TOOLS.iter().map(|(n, _)| *n).collect();
    let mut seen = BTreeSet::new();
    for tool in tools {
        let name = tool["name"].as_str().expect("name");
        seen.insert(name);
        if let Some(snap) = snapshot.get(name) {
            assert_eq!(tool["inputSchema"], snap["parameters"], "schema of {name}");
            assert_eq!(
                tool["description"], snap["description"],
                "description of {name}"
            );
        } else {
            assert_eq!(name, "get_workspace_overview");
        }
    }
    assert_eq!(compat.len(), 55);
    assert!(compat.iter().all(|n| seen.contains(n)));
    // Reihenfolge: 55 kompatible, dann Erweiterung; Out-of-Scope-Tools fehlen.
    assert_eq!(tools[55]["name"], "get_workspace_overview");
    assert!(!seen.contains("create_attachment"));
    assert!(!seen.contains("list_releases"));
    assert!(!seen.contains("list_diffs"));
}

#[tokio::test]
async fn tools_list_with_read_scope_hides_write_tools() {
    let router = app();
    let body = rpc(
        &router,
        RO,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .await;
    let names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"get_issue"));
    assert!(names.contains(&"get_workspace_overview"));
    assert!(names.contains(&"search_documentation"));
    assert!(!names.contains(&"save_issue"));
    assert!(!names.contains(&"delete_comment"));
    assert!(names.iter().all(|n| n.starts_with("list_")
        || n.starts_with("get_")
        || n.ends_with("_images")
        || *n == "search_documentation"));
}

#[tokio::test]
async fn unknown_tool_is_method_not_found() {
    let router = app();
    let resp = rpc(&router, RW, call(5, "list_releases", json!({}))).await;
    assert_eq!(resp["id"], 5);
    assert_eq!(resp["error"]["code"], -32601);
    assert!(resp.get("result").is_none());
    let resp = rpc(
        &router,
        RW,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "nope" }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn invalid_arguments_yield_invalid_request_with_path() {
    let router = app();
    let resp = rpc(&router, RW, call(1, "list_teams", json!({ "limit": 300 }))).await;
    let err = tool_error(&resp);
    assert_eq!(err["error"], "invalid_request");
    assert_eq!(err["status"], 400);
    assert_eq!(err["details"]["path"], "/limit");
    assert!(err["requestId"].as_str().expect("id").starts_with("req_"));

    let resp = rpc(&router, RW, call(2, "get_issue", json!({}))).await;
    assert_eq!(tool_error(&resp)["error"], "invalid_request");

    let resp = rpc(
        &router,
        RW,
        call(3, "list_teams", json!({ "unknown": true })),
    )
    .await;
    assert_eq!(tool_error(&resp)["error"], "invalid_request");
}

#[tokio::test]
async fn non_object_arguments_are_invalid_params() {
    let router = app();
    let resp = rpc(&router, RW, call(1, "list_teams", json!([1]))).await;
    assert_eq!(resp["error"]["code"], -32602);
    let resp = rpc(
        &router,
        RW,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {} }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn write_tool_with_read_token_is_forbidden() {
    let router = app();
    let resp = rpc(
        &router,
        RO,
        call(1, "save_issue", json!({ "title": "x", "team": "ENG" })),
    )
    .await;
    let err = tool_error(&resp);
    assert_eq!(err["error"], "forbidden");
    assert_eq!(err["status"], 403);
    assert_eq!(err["details"]["requiredScope"], "write");
}

#[tokio::test]
async fn valid_call_reaches_handler_and_maps_its_error() {
    // Die Scaffold-Handler liefern `not implemented` (internal) – der Pfad Dispatch → isError.
    let router = app();
    let resp = rpc(&router, RW, call(1, "get_issue", json!({ "id": "ENG-1" }))).await;
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    let err = tool_error(&resp);
    assert_eq!(err["error"], "internal");
    assert_eq!(err["message"], "not implemented");
    assert_eq!(err["status"], 500);
}

#[tokio::test]
async fn notifications_are_accepted_without_body() {
    let router = app();
    let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let (status, resp) = send(
        &router,
        Method::POST,
        "/mcp",
        Some(RW),
        None,
        &msg.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn parse_error_and_invalid_request() {
    let router = app();
    let (status, resp) = send(&router, Method::POST, "/mcp", Some(RW), None, "{not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], -32700);

    let resp = rpc(&router, RW, json!({ "id": 1, "method": "ping" })).await;
    assert_eq!(resp["error"]["code"], -32600);
    let resp = rpc(&router, RW, json!([])).await;
    assert_eq!(resp["error"]["code"], -32600);
}

#[tokio::test]
async fn ping_and_batch() {
    let router = app();
    let resp = rpc(
        &router,
        RW,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
    )
    .await;
    assert_eq!(resp["result"], json!({}));
    let batch = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": 2, "method": "ping" }
    ]);
    let resp = rpc(&router, RW, batch).await;
    assert_eq!(resp.as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn sse_when_client_accepts_only_event_stream() {
    let router = app();
    let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    let (status, resp) = send(
        &router,
        Method::POST,
        "/mcp",
        Some(RW),
        Some("text/event-stream"),
        &msg.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    assert!(text.starts_with("event: message\ndata: {"), "{text}");
    assert!(text.ends_with("\n\n"));
}

#[tokio::test]
async fn get_is_405_and_delete_needs_auth() {
    let router = app();
    let (status, resp) = send(&router, Method::GET, "/mcp", Some(RW), None, "").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(resp.headers().contains_key(header::ALLOW));
    let (status, _) = send(&router, Method::DELETE, "/mcp", None, None, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&router, Method::DELETE, "/mcp", Some(RW), None, "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn unsupported_protocol_header_is_rejected() {
    let router = app();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {RW}"))
        .header("mcp-protocol-version", "1999-01-01")
        .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
        .expect("request");
    let resp = router.clone().oneshot(req).await.expect("response");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn well_known_metadata_is_public() {
    let router = app();
    let (status, resp) = send(
        &router,
        Method::GET,
        "/.well-known/oauth-protected-resource",
        None,
        None,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["resource"], "http://localhost:8080/mcp");
    assert_eq!(body["scopes_supported"], json!(["read", "write"]));

    let (status, resp) = send(
        &router,
        Method::GET,
        "/.well-known/oauth-authorization-server",
        None,
        None,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        body["registration_endpoint"],
        "http://localhost:8080/oauth/register"
    );
}
