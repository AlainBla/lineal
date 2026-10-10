//! Der zusammengesetzte Router deckt alle Pfade aus docs/11 §3.4 ab (kein 404).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use lineal_server::build_router;
use lineal_testkit::fake_app;
use tower::ServiceExt;

#[tokio::test]
async fn every_documented_path_is_routed() {
    let cases = [
        (Method::POST, "/graphql", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (Method::POST, "/mcp", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/mcp", StatusCode::NOT_IMPLEMENTED),
        (
            Method::GET,
            "/.well-known/oauth-protected-resource",
            StatusCode::NOT_IMPLEMENTED,
        ),
        (
            Method::GET,
            "/.well-known/oauth-authorization-server",
            StatusCode::NOT_IMPLEMENTED,
        ),
        (Method::POST, "/oauth/register", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/oauth/authorize", StatusCode::NOT_IMPLEMENTED),
        (Method::POST, "/oauth/token", StatusCode::NOT_IMPLEMENTED),
        (Method::POST, "/auth/login", StatusCode::NOT_IMPLEMENTED),
        (Method::POST, "/auth/logout", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/auth/me", StatusCode::NOT_IMPLEMENTED),
        (Method::PUT, "/uploads/t", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/files/t", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/sync", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/healthz", StatusCode::OK),
        (Method::GET, "/readyz", StatusCode::OK),
    ];
    for (method, uri, expected) in cases {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let res = build_router(fake_app(), false).oneshot(req).await.unwrap();
        assert_eq!(res.status(), expected, "{uri}");
    }
}
