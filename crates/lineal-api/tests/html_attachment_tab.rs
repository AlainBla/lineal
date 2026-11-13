#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Weg B (docs/superpowers/specs/2026-11-13-html-anhang-tab.md): ein `text/html`-Anhang wird
//! NUR auf der konfigurierten, cookielosen Datei-Origin inline + script-fähig ausgeliefert, auf
//! der App-Origin bleibt er ein Download. Ohne `files_base_url` gilt überall das alte Verhalten.

use axum::body::Body;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, HOST};
use axum::http::{Request, StatusCode};
use lineal_api::blob::{GET_TTL, LocalBlobStore};
use lineal_db::PgStore;
use lineal_testkit::test_app;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const DECK: &[u8] = b"<!doctype html><title>Deck</title><script>document.title='x'</script>";

/// Baut eine App mit eigener Datei-Origin (sofern `files_base_url` gesetzt), legt ein HTML-Objekt
/// ab und liefert Router + signierten `/files/<token>`-Pfad.
async fn setup(pool: sqlx::PgPool, files_base_url: Option<&str>) -> (axum::Router, String) {
    let dir = std::env::temp_dir().join(format!("lineal-htmltab-{}", Uuid::now_v7()));
    let mut app = Arc::try_unwrap(test_app(Arc::new(PgStore::new(pool))))
        .ok()
        .unwrap();
    app.config.blob_dir = dir;
    app.config.base_url = "http://lineal.test".into();
    app.config.files_base_url = files_base_url.map(Into::into);
    app.blobs = Arc::new(LocalBlobStore::from_app(&app));
    let app = Arc::new(app);
    app.blobs
        .put_bytes("acme/html/deck", "text/html", DECK.to_vec())
        .await
        .unwrap();
    let url = app
        .blobs
        .signed_get_url("acme/html/deck", GET_TTL)
        .await
        .unwrap();
    let path = url.trim_start_matches("http://lineal.test").to_owned();
    (lineal_api::router::router(app.clone(), false), path)
}

async fn get_with_host(
    router: &axum::Router,
    path: &str,
    host: &str,
) -> (StatusCode, String, String) {
    let res = router
        .clone()
        .oneshot(
            Request::get(path)
                .header(HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let disp = header(&res, CONTENT_DISPOSITION);
    let csp = header(&res, CONTENT_SECURITY_POLICY);
    (status, disp, csp)
}

fn header(res: &axum::response::Response, name: axum::http::HeaderName) -> String {
    res.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn html_opens_inline_on_the_files_origin_but_downloads_on_the_app_origin(pool: sqlx::PgPool) {
    let (router, path) = setup(pool, Some("http://lineal-files.test")).await;

    // Datei-Origin: inline + script-fähige Präsentations-CSP (opake Sandbox).
    let (status, disp, csp) = get_with_host(&router, &path, "lineal-files.test").await;
    assert_eq!(status, StatusCode::OK);
    assert!(disp.starts_with("inline"), "disp={disp}");
    assert!(csp.contains("sandbox allow-scripts"), "csp={csp}");
    assert!(csp.contains("script-src 'unsafe-inline'"), "csp={csp}");
    assert!(!csp.contains("allow-same-origin"), "csp={csp}");

    // App-Origin: unverändert Download + strikte Sandbox, nie ein Skript.
    let (status, disp, csp) = get_with_host(&router, &path, "lineal.test").await;
    assert_eq!(status, StatusCode::OK);
    assert!(disp.starts_with("attachment"), "disp={disp}");
    assert_eq!(csp, "default-src 'none'; sandbox");

    // Review-Focus 4: Spoofing-Host matcht die Datei-Origin nicht -> Download.
    let (_, disp, csp) = get_with_host(&router, &path, "lineal-files.test.evil.com").await;
    assert!(disp.starts_with("attachment"), "disp={disp}");
    assert_eq!(csp, "default-src 'none'; sandbox");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn files_origin_serves_only_files_everything_else_is_404(pool: sqlx::PgPool) {
    // Review-Focus 6 (App-Guard): Auf der Datei-Origin existiert nur /files/* – /healthz & Co.
    // sind 404, obwohl die App sie sonst beantwortet. So wird dort nie ein Cookie gesetzt.
    let (router, files_path) = setup(pool, Some("http://lineal-files.test")).await;

    // Auf der App-Origin liefert /healthz normal 200.
    let (status, ..) = get_with_host(&router, "/healthz", "lineal.test").await;
    assert_eq!(status, StatusCode::OK);
    // Auf der Datei-Origin wird dieselbe Route vom Guard auf 404 gesetzt.
    let (status, ..) = get_with_host(&router, "/healthz", "lineal-files.test").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // /files/* bleibt auf der Datei-Origin erreichbar.
    let (status, ..) = get_with_host(&router, &files_path, "lineal-files.test").await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn without_files_base_url_html_always_downloads(pool: sqlx::PgPool) {
    // Review-Focus 5: Feature aus -> auch unter dem Datei-Host bleibt es das alte Verhalten.
    let (router, path) = setup(pool, None).await;
    let (status, disp, csp) = get_with_host(&router, &path, "lineal-files.test").await;
    assert_eq!(status, StatusCode::OK);
    assert!(disp.starts_with("attachment"), "disp={disp}");
    assert_eq!(csp, "default-src 'none'; sandbox");
}
