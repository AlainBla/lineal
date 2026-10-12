//! axum-Router mit allen Pfaden des API-Adapters (docs/11 §3.4). Gemeinsame Datei: nur S1 / Orchestrator.
//! Nicht implementierte Pfade antworten `501`.

use crate::graphql::{LinealSchema, build_schema};
use crate::middleware::auth::{CurrentSession, MaybeAuth};
use crate::middleware::{self, gql};
use crate::rate_limit::RateHandle;
use crate::{blob, oauth, rest};
use async_graphql::http::GraphiQLSource;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use lineal_app::App;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
struct ApiState {
    app: Arc<App>,
    schema: LinealSchema,
    dev: bool,
}

impl axum::extract::FromRef<ApiState> for Arc<App> {
    fn from_ref(state: &ApiState) -> Self {
        state.app.clone()
    }
}

/// Baut den Router samt Plattform-Middleware. `dev = true` schaltet GraphiQL auf
/// `GET /graphql` und CORS für den Vite-Dev-Server frei.
pub fn router(app: Arc<App>, dev: bool) -> Router {
    // Request-ID, Security-Header, CORS (dev), Auth, Rate-Limit (WP03).
    middleware::apply(routes(app.clone(), dev), app, dev)
}

/// Alle Routen ohne Middleware (für Tests mit eigener `PlatformConfig`).
pub fn routes(app: Arc<App>, dev: bool) -> Router {
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
        .route("/invite/accept", post(rest::auth::accept_invite))
        .route("/uploads/{token}", put(blob::upload))
        .route("/files/{token}", get(blob::download))
        .route("/oauth/register", post(oauth::register))
        .route(
            "/oauth/authorize",
            get(oauth::authorize).post(oauth::authorize_post),
        )
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

async fn graphql_post(
    State(state): State<ApiState>,
    MaybeAuth(auth): MaybeAuth,
    rate: Option<Extension<RateHandle>>,
    session: Option<Extension<CurrentSession>>,
    fails: Option<Extension<std::sync::Arc<crate::auth_failures::AuthFailures>>>,
    client: Option<Extension<crate::client_ip::ClientIp>>,
    request: Result<Json<async_graphql::Request>, JsonRejection>,
) -> Response {
    match request {
        Ok(Json(mut request)) => {
            // Kontext für Resolver des persönlichen Accounts (aktuelle Session, Fehlversuchs-Zähler).
            if let Some(Extension(s)) = session {
                request = request.data(s);
            }
            if let Some(Extension(f)) = fails {
                request = request.data(f);
            }
            if let Some(Extension(c)) = client {
                request = request.data(c);
            }
            gql::execute(&state.schema, &auth, rate.map(|Extension(r)| r), request).await
        }
        Err(rejection) => gql::bad_body(&rejection),
    }
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
    async fn oauth_endpoints_are_wired_and_reject_bad_requests() {
        // /oauth/register ohne Body: 400 (RFC 7591), /oauth/token ohne Form: 4xx, authorize: Fehlerseite.
        let (status, body) = call(Method::POST, "/oauth/register", Body::empty(), false).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("invalid_client_metadata"), "{body}");
        let (status, _) = call(Method::GET, "/oauth/authorize", Body::empty(), false).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(Method::POST, "/oauth/revoke", Body::empty(), false).await;
        assert!(status.is_client_error() || status.is_success());
    }

    #[tokio::test]
    async fn blob_endpoints_reject_unsigned_tokens() {
        for (m, uri) in [(Method::PUT, "/uploads/tok"), (Method::GET, "/files/tok")] {
            let (status, body) = call(m, uri, Body::empty(), false).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
            assert!(body.contains("\"error\":\"forbidden\""), "{uri}: {body}");
        }
    }

    #[tokio::test]
    async fn graphql_requires_auth_executes_and_graphiql_is_dev_only() {
        let q = || Body::from(r#"{"query":"{ __typename }"}"#);
        // ohne Token: 401 mit extensions.code
        let (status, body) = call(Method::POST, "/graphql", q(), false).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("\"code\":\"unauthenticated\""), "{body}");

        // mit Token (FakeStore-Principal)
        let principal = lineal_app::ports::repos::Principal {
            kind: lineal_app::ports::repos::CredentialKind::ApiKey,
            org_id: lineal_domain::ids::OrgId::new_v7(),
            actor: lineal_app::Actor::App {
                id: lineal_domain::ids::UserId::new_v7(),
            },
            scopes: lineal_app::Scopes::read_only(),
            team_scope: None,
        };
        let store = lineal_testkit::FakeStore::new().with_principal(
            crate::middleware::auth::hash_token("lin_api_test"),
            principal,
        );
        let app = lineal_testkit::test_app(Arc::new(store));
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .header("authorization", "lin_api_test")
            .body(q())
            .unwrap();
        let res = router(app, false).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("\"__typename\":\"Query\""));

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
