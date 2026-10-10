//! Abbildung `AppError` -> HTTP (JSON nach docs/00 §7) und -> GraphQL (`extensions.code`,
//! docs/09 §1.2). Die Request-ID kommt aus dem Task-Local der Request-ID-Middleware.

use crate::middleware::request_id;
use async_graphql::{Context, ErrorExtensionValues, Value as GqlValue};
use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lineal_app::{AppError, AppResult, ErrorKind};
use serde_json::{Value, json};

/// Meldung für interne Fehler (Details nur im Log, nie in der Antwort).
const INTERNAL_MESSAGE: &str = "Internal server error.";

/// HTTP-Fehlerantwort eines `AppError` im Fehlermodell aus 00 §7.
#[derive(Debug, Clone)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

fn status_of(kind: ErrorKind) -> StatusCode {
    StatusCode::from_u16(kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Meldung, die nach außen geht (interne Fehler werden geloggt und verallgemeinert).
fn public_message(e: &AppError) -> &str {
    if e.kind == ErrorKind::Internal {
        tracing::error!(error = %e.message, "internal error");
        INTERNAL_MESSAGE
    } else {
        &e.message
    }
}

/// Sekunden für `Retry-After` (`details.retryAfter`), nur bei `rate_limited`.
pub fn retry_after(e: &AppError) -> Option<u64> {
    if e.kind != ErrorKind::RateLimited {
        return None;
    }
    e.details.get("retryAfter").and_then(Value::as_u64)
}

/// JSON-Body nach 00 §7.
pub fn error_body(e: &AppError, request_id: &str) -> Value {
    json!({
        "error": e.kind.code(),
        "message": public_message(e),
        "status": e.kind.http_status(),
        "requestId": request_id,
        "details": e.details,
    })
}

fn with_retry_after(mut res: Response, e: &AppError) -> Response {
    if let Some(secs) = retry_after(e)
        && let Ok(v) = HeaderValue::from_str(&secs.to_string())
    {
        res.headers_mut().insert(header::RETRY_AFTER, v);
    }
    res
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let e = self.0;
        let rid = request_id::current();
        let res = (status_of(e.kind), Json(error_body(&e, &rid))).into_response();
        with_retry_after(res, &e)
    }
}

/// GraphQL-Fehler mit `extensions { code, status, requestId, details }` (09 §1.2).
pub fn gql_error(e: &AppError, request_id: &str) -> async_graphql::Error {
    let mut err = async_graphql::Error::new(public_message(e));
    let mut ext = ErrorExtensionValues::default();
    ext.set("code", e.kind.code());
    ext.set("status", i32::from(e.kind.http_status()));
    ext.set("requestId", request_id);
    ext.set(
        "details",
        GqlValue::from_json(e.details.clone()).unwrap_or(GqlValue::Null),
    );
    err.extensions = Some(ext);
    err
}

/// Für Resolver: `use_case(..).await.gql(ctx)?` liefert Fehler mit `extensions.code`.
pub trait GqlResultExt<T> {
    fn gql(self, ctx: &Context<'_>) -> async_graphql::Result<T>;
}

impl<T> GqlResultExt<T> for AppResult<T> {
    fn gql(self, ctx: &Context<'_>) -> async_graphql::Result<T> {
        self.map_err(|e| {
            let rid = ctx
                .data_opt::<lineal_app::RequestCtx>()
                .map_or_else(request_id::current, |c| c.request_id.clone());
            gql_error(&e, &rid)
        })
    }
}

/// Transportfehler als GraphQL-Antwort (z. B. 401/429); `data` fehlt.
pub fn graphql_error_response(e: &AppError) -> Response {
    let rid = request_id::current();
    let mut server_error = gql_error(e, &rid).into_server_error(Default::default());
    server_error.locations.clear(); // Transportfehler haben keine Query-Position.
    let body = async_graphql::Response::from_errors(vec![server_error]);
    let res = (status_of(e.kind), Json(body)).into_response();
    with_retry_after(res, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_message_is_not_leaked() {
        let body = error_body(&AppError::internal("db password in dsn"), "req_1");
        assert_eq!(body["message"], INTERNAL_MESSAGE);
    }

    #[test]
    fn retry_after_only_for_rate_limited() {
        let e = AppError::rate_limited("slow").with_details(json!({ "retryAfter": 7 }));
        assert_eq!(retry_after(&e), Some(7));
        assert_eq!(retry_after(&AppError::not_found("x")), None);
    }
}
