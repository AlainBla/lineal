#![allow(clippy::unwrap_used, clippy::expect_used)]
//! B3.10: Beim Shutdown schliesst der Server offene WebSockets mit 1001 ("going away").
//! Eigene Test-Binary: das Shutdown-Signal ist prozessweit und wuerde andere Tests stoeren.

use chrono::{Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use lineal_app::ports::Store;
use lineal_app::ports::repos::auth::NewSession;
use lineal_db::PgStore;
use lineal_sync::{SyncConfig, router_with};
use lineal_testkit::{SeedData, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn open_sync_socket_gets_close_1001_on_shutdown(pool: PgPool) {
    let d = SeedData::acme();
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(d.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.auth()
        .insert_session(&NewSession {
            id: Uuid::now_v7(),
            user_id: d.alice,
            token_hash: lineal_app::usecases::auth::hash_token("session-token-alice"),
            csrf_token_hash: None,
            user_agent: None,
            ip_address: None,
            expires_at: Utc::now() + ChronoDuration::days(1),
            now: Utc::now(),
        })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let router = router_with(
        app,
        SyncConfig {
            poll_interval: Duration::from_millis(40),
            ..SyncConfig::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut req = format!("ws://{addr}/sync").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        "lineal_session=session-token-alice".parse().unwrap(),
    );
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    // Erst nach dem Hello shutdown melden.
    let hello = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap();
    assert!(hello.is_some());
    lineal_sync::begin_shutdown();
    let code = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(m) = ws.next().await {
            if let Ok(Message::Close(Some(frame))) = m {
                return Some(u16::from(frame.code));
            }
        }
        None
    })
    .await
    .unwrap();
    assert_eq!(code, Some(1001));
}
