//! axum-Router mit allen Pfaden des API-Adapters (docs/11 §3.4). Gemeinsame Datei: nur S1 / Orchestrator.
//! Nicht implementierte Pfade antworten `501`.

use crate::graphql::{LinealSchema, build_schema};
use crate::{blob, oauth, rest};
use async_graphql::http::GraphiQLSource;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lineal_app::App;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
struct ApiState {
    app: Arc<App>,
    schema: LinealSchema,
    dev: bool,
}

/// Baut den Router. `dev = true` schaltet GraphiQL auf `GET /graphql` frei.
pub fn router(app: Arc<App>, dev: bool) -> Router {
    let state = ApiState {
        schema: build_schema(app.clone()),
        app,
        dev,
    };
    Router::new()
        .route("/graphql", post(graphql_post).get(graphiql))
        .route("/auth/login", post(rest::auth::login))
        .route("/auth/logout", post(rest::auth::logout))
        .route("/auth/me", get(rest::auth::me))
        .route("/uploads/{token}", put(blob::upload))
        .route("/files/{token}", get(blob::download))
        .route("/oauth/register", post(oauth::register))
        .route("/oauth/authorize", get(oauth::authorize))
        .route("/oauth/token", post(oauth::token))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

async fn graphql_post(
    State(state): State<ApiState>,
    Json(request): Json<async_graphql::Request>,
) -> Json<async_graphql::Response> {
    // WP03: Auth-Middleware setzt den `RequestCtx` und reicht ihn per `request.data(..)` durch.
    Json(state.schema.execute(request).await)
}

async fn graphiql(State(state): State<ApiState>) -> impl IntoResponse {
    if state.dev {
        Html(GraphiQLSource::build().endpoint("/graphql").finish()).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn readyz(State(state): State<ApiState>) -> (StatusCode, Json<serde_json::Value>) {
    match state.app.store.begin_system().await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ready" }))),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable" })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use http_body_util::BodyExt;
    use lineal_testkit::fake_app;
    use tower::ServiceExt;

    async fn call(method: Method, uri: &str, body: Body, dev: bool) -> (StatusCode, String) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        let res = router(fake_app(), dev).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn healthz_and_readyz_are_ok() {
        assert_eq!(
            call(Method::GET, "/healthz", Body::empty(), false).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(Method::GET, "/readyz", Body::empty(), false).await.0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn unimplemented_paths_answer_501_with_error_model() {
        for (m, uri) in [
            (Method::POST, "/auth/login"),
            (Method::POST, "/auth/logout"),
            (Method::GET, "/auth/me"),
            (Method::PUT, "/uploads/tok"),
            (Method::GET, "/files/tok"),
            (Method::POST, "/oauth/register"),
            (Method::GET, "/oauth/authorize"),
            (Method::POST, "/oauth/token"),
        ] {
            let (status, body) = call(m, uri, Body::empty(), false).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{uri}");
            assert!(body.contains("\"error\":\"internal\""), "{uri}: {body}");
        }
    }

    #[tokio::test]
    async fn graphql_executes_and_graphiql_is_dev_only() {
        let q = Body::from(r#"{"query":"{ _orgReady }"}"#);
        let (status, body) = call(Method::POST, "/graphql", q, false).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"_orgReady\":true"), "{body}");
        assert_eq!(
            call(Method::GET, "/graphql", Body::empty(), false).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(Method::GET, "/graphql", Body::empty(), true).await.0,
            StatusCode::OK
        );
    }
}
