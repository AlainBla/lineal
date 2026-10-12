#![allow(clippy::unwrap_used, clippy::expect_used)]
//! HTTP-Adapter gegen einen lokalen Mock-Server (axum). Kein Test kontaktiert das echte Linear.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use lineal_app::usecases::import::Secret;
use lineal_import::port::{PageQuery, SourceError};
use lineal_import::{ImportSource, LinearConfig, LinearSource, SourceEntity, UrlPolicy};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TOKEN: &str = "lin_api_MOCKTOKENabcdef0123456789";

#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<(Option<String>, Value)>>,
    script: Mutex<Vec<Reply>>,
}

enum Reply {
    RateLimited429,
    RateLimitedBody400,
    Unauthorized,
    ServerError,
    Json(Value),
}

async fn graphql(
    State(m): State<Arc<Mock>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    m.calls.lock().unwrap().push((auth, body));
    let reply = {
        let mut s = m.script.lock().unwrap();
        if s.is_empty() {
            Reply::ServerError
        } else {
            s.remove(0)
        }
    };
    let reset = (chrono::Utc::now().timestamp_millis() + 200).to_string();
    match reply {
        Reply::RateLimited429 => {
            let mut r = StatusCode::TOO_MANY_REQUESTS.into_response();
            r.headers_mut().insert(
                "x-ratelimit-requests-reset",
                HeaderValue::from_str(&reset).unwrap(),
            );
            r
        }
        Reply::RateLimitedBody400 => {
            let mut r = (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "errors": [{ "message": "rate limit", "extensions": { "code": "RATELIMITED" } }] })),
            )
                .into_response();
            r.headers_mut().insert(
                "x-ratelimit-requests-reset",
                HeaderValue::from_str(&reset).unwrap(),
            );
            r
        }
        Reply::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
        Reply::ServerError => StatusCode::BAD_GATEWAY.into_response(),
        Reply::Json(v) => {
            let mut r = axum::Json(v).into_response();
            r.headers_mut().insert(
                "x-ratelimit-requests-remaining",
                HeaderValue::from_static("100"),
            );
            r
        }
    }
}

async fn serve(m: Arc<Mock>) -> String {
    let app = Router::new()
        .route("/graphql", post(graphql))
        .route(
            "/file.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![1u8, 2, 3]) }),
        )
        .route(
            "/page.html",
            get(|| async { ([(header::CONTENT_TYPE, "text/html")], "<script>x</script>") }),
        )
        .route(
            "/big.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![0u8; 4096]) }),
        )
        .route(
            "/redir",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "http://127.0.0.1:1/secret")],
                )
            }),
        )
        .with_state(m);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn source(base: &str) -> LinearSource {
    let mut policy = UrlPolicy::for_hosts(vec!["127.0.0.1".into()]);
    policy.allow_local_for_tests = true;
    policy.max_bytes = 1024;
    let cfg = LinearConfig {
        endpoint: format!("{base}/graphql"),
        policy,
        max_retries: 3,
        backoff_base: Duration::from_millis(10),
        max_wait: Duration::from_secs(2),
        ..Default::default()
    };
    LinearSource::with_config(&Secret::new(TOKEN).unwrap(), cfg).unwrap()
}

fn issues_page(ids: &[&str], next: Option<&str>) -> Value {
    json!({ "data": { "issues": {
        "nodes": ids.iter().map(|i| json!({ "id": i, "number": 1, "team": { "id": "t" }, "labels": { "nodes": [{ "id": "l" }] } })).collect::<Vec<_>>(),
        "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next },
    } } })
}

#[tokio::test]
async fn pagination_auth_header_and_flattening() {
    let m = Arc::new(Mock::default());
    *m.script.lock().unwrap() = vec![
        Reply::Json(issues_page(&["a", "b"], Some("cur1"))),
        Reply::Json(issues_page(&["c"], None)),
    ];
    let base = serve(m.clone()).await;
    let src = source(&base);
    let items = src.collect(SourceEntity::Issues, None, 10).await.unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0]["labels"],
        json!([{ "id": "l" }]),
        "connections are flattened"
    );
    let calls = m.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(
        calls.iter().all(|(a, _)| a.as_deref() == Some(TOKEN)),
        "personal key without Bearer"
    );
    assert_eq!(calls[0].1["variables"]["after"], Value::Null);
    assert_eq!(calls[1].1["variables"]["after"], "cur1");
    assert!(
        calls
            .iter()
            .all(|(_, b)| !b["query"].as_str().unwrap().contains("mutation"))
    );
}

#[tokio::test]
async fn rate_limits_are_waited_out_and_errors_never_contain_the_token() {
    let m = Arc::new(Mock::default());
    *m.script.lock().unwrap() = vec![
        Reply::RateLimited429,
        Reply::RateLimitedBody400,
        Reply::ServerError,
        Reply::Json(issues_page(&["a"], None)),
    ];
    let base = serve(m.clone()).await;
    let src = source(&base);
    let started = std::time::Instant::now();
    let page = src
        .page(&PageQuery {
            entity: SourceEntity::Issues,
            cursor: None,
            updated_after: None,
            scope: Default::default(),
            limit: 50,
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        m.calls.lock().unwrap().len(),
        4,
        "429, RATELIMITED, 5xx are retried"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "waited for the reset"
    );

    // 401 -> Auth ohne Token im Text; keine Wiederholung.
    *m.script.lock().unwrap() = vec![Reply::Unauthorized];
    let err = src.workspace().await.unwrap_err();
    assert_eq!(err, SourceError::Auth);
    assert!(!format!("{err} {err:?}").contains("MOCKTOKEN"));

    // Dauerhafter Serverfehler: nach den Wiederholungen aufgeben, ohne Antwortinhalt/Token.
    m.script.lock().unwrap().clear();
    let err = src.workspace().await.unwrap_err();
    assert!(matches!(err, SourceError::Transport(_)));
    assert!(!err.to_string().contains("MOCKTOKEN"));
}

#[tokio::test]
async fn downloads_enforce_size_type_and_never_follow_redirects() {
    let m = Arc::new(Mock::default());
    let base = serve(m).await;
    let src = source(&base);
    let ok = src.download(&format!("{base}/file.png")).await.unwrap();
    assert_eq!(
        (ok.bytes, ok.content_type.as_str()),
        (vec![1, 2, 3], "image/png")
    );
    for (path, what) in [
        ("/page.html", "content type"),
        ("/big.png", "size"),
        ("/redir", "redirect"),
    ] {
        let e = src.download(&format!("{base}{path}")).await.unwrap_err();
        assert!(matches!(e, SourceError::Rejected(_)), "{what}: {e:?}");
    }
    // Fremde Hosts und Metadaten-Adressen werden gar nicht erst angefragt.
    for url in [
        "https://evil.example.com/a.png",
        "http://169.254.169.254/latest",
        "https://10.0.0.5/a.png",
    ] {
        let e = src.download(url).await.unwrap_err();
        assert!(matches!(e, SourceError::Rejected(_)), "{url}");
    }
}
