#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP15: WebSocket-Delta-Strom gegen echte Datenbank und echten Socket (Seed `acme`).
//! Aktionen werden direkt ins `sync_log` geschrieben; die Event→Aktion-Abbildung testet `lineal-jobs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use lineal_app::App;
use lineal_app::ports::Store;
use lineal_app::ports::repos::auth::NewSession;
use lineal_app::ports::repos::outbox::NewSyncAction;
use lineal_db::PgStore;
use lineal_domain::events::EventOrigin;
use lineal_domain::ids::EventId;
use lineal_sync::{SyncConfig, router_with};
use lineal_testkit::{SeedData, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

struct Env {
    app: Arc<App>,
    pool: PgPool,
    addr: SocketAddr,
    d: SeedData,
}

fn token(name: &str) -> String {
    format!("session-token-{name}")
}

async fn env_with(pool: PgPool, cfg: impl FnOnce(&PgPool) -> SyncConfig) -> Env {
    let d = SeedData::acme();
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(d.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    for (name, user) in [("alice", d.alice), ("bob", d.bob), ("carol", d.carol)] {
        tx.auth()
            .insert_session(&NewSession {
                id: Uuid::now_v7(),
                user_id: user,
                token_hash: lineal_app::usecases::auth::hash_token(&token(name)),
                csrf_token_hash: None,
                user_agent: None,
                ip_address: None,
                expires_at: Utc::now() + ChronoDuration::days(1),
                now: Utc::now(),
            })
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let router = router_with(app.clone(), cfg(&pool));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Env { app, pool, addr, d }
}

fn fast() -> SyncConfig {
    SyncConfig {
        poll_interval: Duration::from_millis(40),
        ..SyncConfig::default()
    }
}

async fn env(pool: PgPool) -> Env {
    env_with(pool, |_| fast()).await
}

impl Env {
    /// Schreibt eine Aktion ins Log; liefert ihre Stream-ID.
    async fn append(&self, action: &str, model: &str, groups: Vec<String>, data: Value) -> i64 {
        let mut tx = self.app.store.begin(self.d.org).await.unwrap();
        let id = tx
            .outbox()
            .sync_append(&[NewSyncAction {
                event_id: EventId::new_v7(),
                n: 0,
                sync_id: 1,
                action: action.into(),
                model_name: model.into(),
                model_id: Uuid::now_v7(),
                data,
                groups,
                origin: EventOrigin::Import,
                actor_id: None,
            }])
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn connect(&self, who: &str, since: Option<i64>) -> Ws {
        self.try_connect(Some(who), since, &[]).await.unwrap()
    }

    async fn try_connect(
        &self,
        who: Option<&str>,
        since: Option<i64>,
        headers: &[(&str, &str)],
    ) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let query = since.map(|s| format!("?since={s}")).unwrap_or_default();
        let mut req = format!("ws://{}/sync{query}", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(who) = who {
            req.headers_mut().insert(
                "cookie",
                format!("lineal_session={}", token(who)).parse().unwrap(),
            );
        }
        for (k, v) in headers {
            req.headers_mut().insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        let stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        tokio_tungstenite::client_async(req, stream)
            .await
            .map(|(ws, _)| ws)
    }
}

/// Nächste JSON-Nachricht (Ping/Pong werden übersprungen); `None` bei Timeout/Close.
async fn next_json(ws: &mut Ws, wait: Duration) -> Option<Value> {
    let fut = async {
        loop {
            match ws.next().await? {
                Ok(Message::Text(t)) => return serde_json::from_str::<Value>(&t).ok(),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    };
    tokio::time::timeout(wait, fut).await.ok().flatten()
}

/// Nächste `sync`-Nachricht (überspringt `hello`/`pong`).
async fn next_sync(ws: &mut Ws, wait: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let msg = next_json(ws, left).await?;
        if msg["type"] == "sync" {
            return Some(msg);
        }
    }
}

fn model_ids(msg: &Value) -> Vec<String> {
    msg["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["data"]["tag"].as_str().unwrap().to_owned())
        .collect()
}

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(500);

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn unauthenticated_upgrade_is_401(pool: PgPool) {
    let e = env(pool).await;
    // ohne Zugangsdaten
    match e.try_connect(None, None, &[]).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }
    // ungültiges Token
    let mut req = format!("ws://{}/sync", e.addr)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("cookie", "lineal_session=nope".parse().unwrap());
    let stream = tokio::net::TcpStream::connect(e.addr).await.unwrap();
    match tokio_tungstenite::client_async(req, stream).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        other => panic!("expected 401, got {:?}", other.map(|_| ())),
    }
    // normaler GET ohne Upgrade-Header und ohne Auth: ebenfalls 401 (nicht 400)
    let router = router_with(e.app.clone(), fast());
    let res = router
        .oneshot(Request::get("/sync").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_actions_reach_only_members_even_after_removal(pool: PgPool) {
    let e = env(pool).await;
    let des = format!("team:{}", e.d.des.0);
    let eng = format!("team:{}", e.d.eng.0);

    // Vorbedingung: Carol (Gast in WEB) sieht DES nicht, Bob (Mitglied) schon.
    let mut bob = e.connect("bob", None).await;
    let mut carol = e.connect("carol", None).await;
    assert_eq!(next_json(&mut bob, WAIT).await.unwrap()["type"], "hello");
    assert_eq!(next_json(&mut carol, WAIT).await.unwrap()["type"], "hello");

    e.append("U", "Issue", vec![des.clone()], json!({ "tag": "des-1" }))
        .await;
    let got = next_sync(&mut bob, WAIT).await.unwrap();
    assert_eq!(model_ids(&got), ["des-1"]);
    assert_eq!(got["actions"][0]["syncGroups"], json!([des]));
    assert!(
        next_sync(&mut carol, QUIET).await.is_none(),
        "Nicht-Mitglied darf keine DES-Aktion sehen"
    );

    // Bob wird aus DES entfernt: weitere DES-Aktionen erreichen ihn nicht mehr.
    sqlx::query("DELETE FROM team_membership WHERE team_id = $1 AND user_id = $2")
        .bind(e.d.des.0)
        .bind(e.d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.append("U", "Issue", vec![des.clone()], json!({ "tag": "des-2" }))
        .await;
    e.append("U", "Issue", vec![eng.clone()], json!({ "tag": "eng-1" }))
        .await;
    let got = next_sync(&mut bob, WAIT).await.unwrap();
    assert_eq!(
        model_ids(&got),
        ["eng-1"],
        "nach dem Entfernen nur noch ENG"
    );
    assert!(next_sync(&mut bob, QUIET).await.is_none());

    // Auch ein Reconnect mit altem `since` holt die DES-Lücke nicht nach.
    let mut bob2 = e.connect("bob", Some(0)).await;
    let mut all = vec![];
    while let Some(m) = next_sync(&mut bob2, QUIET).await {
        all.extend(model_ids(&m));
    }
    assert!(!all.iter().any(|t| t.starts_with("des")), "{all:?}");
    assert!(all.contains(&"eng-1".to_owned()));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn notification_actions_only_reach_their_owner(pool: PgPool) {
    let e = env(pool).await;
    let mut alice = e.connect("alice", None).await;
    let mut bob = e.connect("bob", None).await;
    next_json(&mut alice, WAIT).await.unwrap();
    next_json(&mut bob, WAIT).await.unwrap();

    e.append(
        "I",
        "Notification",
        vec![format!("user:{}", e.d.bob.0)],
        json!({ "tag": "n-bob" }),
    )
    .await;
    e.append(
        "U",
        "Notification",
        vec![format!("user:{}", e.d.alice.0)],
        json!({ "tag": "n-alice" }),
    )
    .await;
    assert_eq!(
        model_ids(&next_sync(&mut bob, WAIT).await.unwrap()),
        ["n-bob"]
    );
    assert_eq!(
        model_ids(&next_sync(&mut alice, WAIT).await.unwrap()),
        ["n-alice"]
    );
    assert!(next_sync(&mut bob, QUIET).await.is_none());
    assert!(next_sync(&mut alice, QUIET).await.is_none());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reconnect_with_since_delivers_the_gap_in_order(pool: PgPool) {
    let e = env(pool).await;
    let eng = vec![format!("team:{}", e.d.eng.0)];
    let mut ws = e.connect("bob", None).await;
    assert_eq!(next_json(&mut ws, WAIT).await.unwrap()["type"], "hello");

    e.append("I", "Issue", eng.clone(), json!({ "tag": "a1" }))
        .await;
    let first = next_sync(&mut ws, WAIT).await.unwrap();
    let last_sync_id = first["lastSyncId"].as_i64().unwrap();
    assert_eq!(first["actions"][0]["id"].as_i64().unwrap(), last_sync_id);
    drop(ws);

    // Lücke: während der Client offline ist
    let a2 = e
        .append("U", "Issue", eng.clone(), json!({ "tag": "a2" }))
        .await;
    let a3 = e
        .append("U", "Issue", eng.clone(), json!({ "tag": "a3" }))
        .await;
    assert!(a2 > last_sync_id && a3 > a2);

    let mut ws = e.connect("bob", Some(last_sync_id)).await;
    let hello = next_json(&mut ws, WAIT).await.unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["resumed"], true);
    let got = next_sync(&mut ws, WAIT).await.unwrap();
    assert_eq!(model_ids(&got), ["a2", "a3"]);
    assert_eq!(got["lastSyncId"].as_i64().unwrap(), a3);

    // danach live
    e.append("U", "Issue", eng, json!({ "tag": "a4" })).await;
    assert_eq!(model_ids(&next_sync(&mut ws, WAIT).await.unwrap()), ["a4"]);

    // Ping/Pong liefert den Cursor
    ws.send(Message::Text(json!({ "type": "ping" }).to_string().into()))
        .await
        .unwrap();
    let pong = next_json(&mut ws, WAIT).await.unwrap();
    assert_eq!(pong["type"], "pong");
    assert!(pong["lastSyncId"].as_i64().unwrap() > a3);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn too_old_or_future_since_requires_resync(pool: PgPool) {
    let e = env(pool).await;
    let eng = vec![format!("team:{}", e.d.eng.0)];
    let a1 = e
        .append("I", "Issue", eng.clone(), json!({ "tag": "a1" }))
        .await;
    let a2 = e
        .append("I", "Issue", eng.clone(), json!({ "tag": "a2" }))
        .await;

    // Aufbewahrung abgelaufen: Log wird geleert, Wasserzeichen steht auf a2.
    let mut tx = e.app.store.begin_system().await.unwrap();
    let purged = tx
        .outbox()
        .sync_purge(Utc::now() + ChronoDuration::days(1))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(purged, 2);
    let a3 = e.append("I", "Issue", eng, json!({ "tag": "a3" })).await;

    // since vor dem Wasserzeichen → resetRequired + Close
    let mut ws = e.connect("bob", Some(a1)).await;
    assert_eq!(
        next_json(&mut ws, WAIT).await.unwrap(),
        json!({ "type": "resetRequired" })
    );
    assert!(next_json(&mut ws, WAIT).await.is_none());

    // since == Wasserzeichen: lückenlos, nur a3 fehlt
    let mut ws = e.connect("bob", Some(a2)).await;
    assert_eq!(next_json(&mut ws, WAIT).await.unwrap()["type"], "hello");
    assert_eq!(model_ids(&next_sync(&mut ws, WAIT).await.unwrap()), ["a3"]);

    // since aus der Zukunft (z. B. zurückgesetzte Datenbank) → resetRequired
    let mut ws = e.connect("bob", Some(a3 + 1000)).await;
    assert_eq!(
        next_json(&mut ws, WAIT).await.unwrap()["type"],
        "resetRequired"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn limits_origin_and_chunking(pool: PgPool) {
    let e = env_with(pool, |_| SyncConfig {
        max_connections_per_user: 1,
        max_outgoing_bytes: 200,
        ..fast()
    })
    .await;
    // Verbindungslimit je User
    let mut first = e.connect("bob", None).await;
    match e.try_connect(Some("bob"), None, &[]).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
            assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        }
        other => panic!("expected 429, got {:?}", other.map(|_| ())),
    }
    // anderer User ist nicht betroffen
    let _alice = e.connect("alice", None).await;
    // Fremder Origin (Cross-Site-WebSocket-Hijacking) → 403
    match e
        .try_connect(Some("alice"), None, &[("origin", "https://evil.example")])
        .await
    {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => {
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
        }
        other => panic!("expected 403, got {:?}", other.map(|_| ())),
    }
    // Pakete werden nach Größe zerlegt
    next_json(&mut first, WAIT).await.unwrap();
    let eng = vec![format!("team:{}", e.d.eng.0)];
    let mut tx = e.app.store.begin(e.d.org).await.unwrap();
    let ev = EventId::new_v7();
    let rows: Vec<NewSyncAction> = (0..4)
        .map(|n| NewSyncAction {
            event_id: ev,
            n,
            sync_id: 1,
            action: "U".into(),
            model_name: "Issue".into(),
            model_id: Uuid::now_v7(),
            data: json!({ "tag": format!("c{n}"), "pad": "x".repeat(120) }),
            groups: eng.clone(),
            origin: EventOrigin::User,
            actor_id: None,
        })
        .collect();
    tx.outbox().sync_append(&rows).await.unwrap();
    // Wiederholung desselben Events ist idempotent
    assert_eq!(tx.outbox().sync_append(&rows).await.unwrap(), None);
    tx.commit().await.unwrap();
    let mut tags = vec![];
    let mut messages = 0;
    while tags.len() < 4 {
        let m = next_sync(&mut first, WAIT).await.unwrap();
        messages += 1;
        tags.extend(model_ids(&m));
    }
    assert_eq!(tags, ["c0", "c1", "c2", "c3"]);
    assert!(messages >= 2, "Paket wurde nicht zerlegt");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn notify_wakes_connections_without_polling(pool: PgPool) {
    let e = env_with(pool, |pool| SyncConfig {
        poll_interval: Duration::from_secs(3600),
        listen_pool: Some(pool.clone()),
        ..SyncConfig::default()
    })
    .await;
    let mut ws = e.connect("bob", None).await;
    assert_eq!(next_json(&mut ws, WAIT).await.unwrap()["type"], "hello");
    // Dem LISTEN-Task Zeit zum Verbinden geben.
    tokio::time::sleep(Duration::from_millis(500)).await;
    e.append(
        "U",
        "Issue",
        vec![format!("team:{}", e.d.eng.0)],
        json!({ "tag": "wake" }),
    )
    .await;
    assert_eq!(
        model_ids(&next_sync(&mut ws, WAIT).await.unwrap()),
        ["wake"]
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn silent_clients_are_dropped(pool: PgPool) {
    let e = env_with(pool, |_| SyncConfig {
        ping_interval: Duration::from_millis(50),
        idle_timeout: Duration::from_millis(120),
        ..fast()
    })
    .await;
    let mut ws = e.connect("bob", None).await;
    // Der Client liest nicht (antwortet also auch nicht auf Pings) → Server beendet.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mut saw_close = false;
    while let Ok(Some(msg)) = tokio::time::timeout(WAIT, ws.next()).await {
        // Antwortet der Client auf ein Ping, scheitert das Schreiben an der beendeten Verbindung.
        let Ok(msg) = msg else {
            saw_close = true;
            break;
        };
        if matches!(msg, Message::Close(_)) {
            saw_close = true;
            break;
        }
    }
    assert!(saw_close, "Server hat den stummen Client nicht getrennt");
}

/// Wartet auf den Close-Frame und liefert den Code (`None` bei Timeout/ohne Close-Frame).
async fn next_close(ws: &mut Ws, wait: Duration) -> Option<u16> {
    let fut = async {
        loop {
            match ws.next().await? {
                Ok(Message::Close(frame)) => return frame.map(|f| u16::from(f.code)),
                Err(_) => return None,
                Ok(_) => {}
            }
        }
    };
    tokio::time::timeout(wait, fut).await.ok().flatten()
}

fn revalidating(pool: &PgPool, every: Duration) -> SyncConfig {
    let _ = pool;
    SyncConfig {
        revalidate_interval: every,
        ..fast()
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn revoked_session_is_closed_with_4401_by_the_periodic_check(pool: PgPool) {
    let e = env_with(pool, |p| revalidating(p, Duration::from_millis(150))).await;
    let mut bob = e.connect("bob", None).await;
    let mut alice = e.connect("alice", None).await;
    assert_eq!(next_json(&mut bob, WAIT).await.unwrap()["type"], "hello");
    assert_eq!(next_json(&mut alice, WAIT).await.unwrap()["type"], "hello");
    sqlx::query("UPDATE user_session SET revoked_at = now() WHERE token_hash = $1")
        .bind(lineal_app::usecases::auth::hash_token(&token("bob")))
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(next_close(&mut bob, WAIT).await, Some(4401));
    // Alices Verbindung bleibt unberührt und liefert weiter.
    let eng = format!("team:{}", e.d.eng.0);
    e.append("U", "Issue", vec![eng], json!({ "tag": "still-alive" }))
        .await;
    assert_eq!(
        model_ids(&next_sync(&mut alice, WAIT).await.unwrap()),
        ["still-alive"]
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn suspended_user_is_closed_with_4401(pool: PgPool) {
    let e = env_with(pool, |p| revalidating(p, Duration::from_millis(150))).await;
    let mut carol = e.connect("carol", None).await;
    assert_eq!(next_json(&mut carol, WAIT).await.unwrap()["type"], "hello");
    sqlx::query("UPDATE \"user\" SET active = false WHERE id = $1")
        .bind(e.d.carol.0)
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(next_close(&mut carol, WAIT).await, Some(4401));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn membership_change_triggers_an_immediate_recheck(pool: PgPool) {
    // Periodische Prüfung praktisch aus (1 h): nur die Zustellung einer User-Änderung prüft.
    let e = env_with(pool, |p| revalidating(p, Duration::from_secs(3600))).await;
    let mut bob = e.connect("bob", None).await;
    assert_eq!(next_json(&mut bob, WAIT).await.unwrap()["type"], "hello");
    sqlx::query("UPDATE user_session SET revoked_at = now() WHERE token_hash = $1")
        .bind(lineal_app::usecases::auth::hash_token(&token("bob")))
        .execute(&e.pool)
        .await
        .unwrap();
    // Ohne Mitgliedschafts-Ereignis bleibt die Verbindung offen ...
    assert!(next_close(&mut bob, QUIET).await.is_none());
    // ... eine zugestellte User-Änderung löst die Prüfung aus.
    e.append(
        "U",
        "User",
        vec!["org".into()],
        json!({ "tag": "member-changed" }),
    )
    .await;
    assert_eq!(next_close(&mut bob, WAIT).await, Some(4401));
}
