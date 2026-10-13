//! Request-ID und Tracing: jede Anfrage bekommt `req_<uuid>`; die ID steht im Tracing-Span,
//! im Header `X-Request-Id`, im `RequestCtx` und in jedem Fehlerbody (Task-Local).

use axum::extract::Request;
use axum::http::{HeaderValue, header::HeaderName};
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;

pub const HEADER: HeaderName = HeaderName::from_static("x-request-id");

tokio::task_local! {
    static REQUEST_ID: String;
}

/// Request-ID als Request-Extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

/// ID der laufenden Anfrage (außerhalb einer Anfrage: `req_unknown`).
pub fn current() -> String {
    REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "req_unknown".to_owned())
}

pub fn generate() -> String {
    format!("req_{}", Uuid::now_v7().simple())
}

/// Pfad fürs Log: Fähigkeits-URLs (`/files/<Token>`, `/uploads/<Token>`) enthalten ein
/// signiertes Token, das 1-5 min als Zugriffsrecht gilt, und werden nie im Klartext geloggt.
pub fn loggable_path(path: &str) -> String {
    for prefix in ["/files/", "/uploads/"] {
        if path.starts_with(prefix) {
            return format!("{prefix}<redacted>");
        }
    }
    path.to_owned()
}

pub async fn middleware(mut req: Request, next: Next) -> Response {
    let id = generate();
    req.extensions_mut().insert(RequestId(id.clone()));
    let client_ip = req
        .extensions()
        .get::<crate::client_ip::ClientIp>()
        .map(|c| c.0.to_string())
        .unwrap_or_default();
    let span = tracing::info_span!(
        "http",
        request_id = %id,
        client_ip = %client_ip,
        method = %req.method(),
        path = %loggable_path(req.uri().path()),
    );
    let started = Instant::now();
    let mut res = REQUEST_ID
        .scope(id.clone(), next.run(req).instrument(span.clone()))
        .await;
    span.in_scope(|| {
        tracing::info!(
            status = res.status().as_u16(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "request finished"
        );
    });
    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert(HEADER, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::loggable_path;

    #[test]
    fn capability_urls_are_redacted_in_the_log() {
        assert_eq!(loggable_path("/files/abc.def"), "/files/<redacted>");
        assert_eq!(loggable_path("/uploads/abc.def"), "/uploads/<redacted>");
        assert_eq!(loggable_path("/graphql"), "/graphql");
        assert_eq!(loggable_path("/filesystem"), "/filesystem");
    }
}
