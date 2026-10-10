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
        (Method::POST, "/mcp", StatusCode::UNAUTHORIZED),
        (Method::GET, "/mcp", StatusCode::METHOD_NOT_ALLOWED),
        (
            Method::GET,
            "/.well-known/oauth-protected-resource",
            StatusCode::OK,
        ),
        (
            Method::GET,
            "/.well-known/oauth-authorization-server",
            StatusCode::OK,
        ),
        (Method::POST, "/oauth/register", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/oauth/authorize", StatusCode::NOT_IMPLEMENTED),
        (Method::POST, "/oauth/token", StatusCode::NOT_IMPLEMENTED),
        (
            Method::POST,
            "/auth/login",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (Method::POST, "/auth/logout", StatusCode::OK),
        (Method::GET, "/auth/me", StatusCode::UNAUTHORIZED),
        // Blob-Endpunkte (WP03): ungültige Signatur -> 403
        (Method::PUT, "/uploads/t", StatusCode::FORBIDDEN),
        (Method::GET, "/files/t", StatusCode::FORBIDDEN),
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
