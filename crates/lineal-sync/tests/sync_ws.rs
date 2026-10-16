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
    // Der erste Verbindungsaufbau startet den LISTEN-Task mit eigener Verbindung; unter Last
    // dauert "hello" hier länger als WAIT.
    assert_eq!(
        next_json(&mut ws, Duration::from_secs(30)).await.unwrap()["type"],
        "hello"
    );
    // Warten, bis der LISTEN-Task wirklich lauscht (unter Last kann das deutlich länger als
    // ein paar hundert Millisekunden dauern; ein NOTIFY davor ginge verloren).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let listening: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND query ILIKE 'LISTEN%lineal_sync%'",
        )
        .fetch_one(&e.pool)
        .await
        .unwrap();
        if listening > 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "LISTEN-Task hat nicht rechtzeitig gelauscht"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    e.append(
        "U",
        "Issue",
        vec![format!("team:{}", e.d.eng.0)],
        json!({ "tag": "wake" }),
    )
    .await;
    // Unter Last (voller Workspace-Lauf mit nice) braucht der Weg NOTIFY -> Wake -> Delta
    // länger als WAIT; großzügig warten, Polling bleibt bei 3600 s ausgeschlossen.
    assert_eq!(
        model_ids(&next_sync(&mut ws, Duration::from_secs(30)).await.unwrap()),
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

// --- WP32: Freigaben, Gäste, Bootstrap ---------------------------------------------------------

/// Gruppen, wie `lineal-jobs` sie je Modell vergibt (siehe `sync_model`): hier direkt ins Log
/// geschrieben, damit der Adapter-Filter isoliert geprüft wird.
fn g(kind: &str, id: Uuid) -> String {
    format!("{kind}:{id}")
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn guest_receives_workspace_labels_and_own_team_people_but_nothing_foreign(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    let mut carol = e.connect("carol", None).await;
    let mut bob = e.connect("bob", None).await;
    next_json(&mut carol, WAIT).await.unwrap();
    next_json(&mut bob, WAIT).await.unwrap();

    // Alles, was Carol (Gast, nur WEB) NICHT sehen darf.
    e.append(
        "U",
        "Issue",
        vec![g("team", d.eng.0)],
        json!({ "tag": "eng-issue" }),
    )
    .await;
    e.append(
        "U",
        "IssueLabel",
        vec![g("team", d.eng.0)],
        json!({ "tag": "eng-label" }),
    )
    .await;
    e.append(
        "U",
        "Initiative",
        vec!["org".into()],
        json!({ "tag": "initiative" }),
    )
    .await;
    e.append(
        "U",
        "User",
        vec!["org".into(), g("user", d.alice.0), g("tmem", d.eng.0)],
        json!({ "tag": "eng-only-person" }),
    )
    .await;
    // Was sie sehen darf: Workspace-Label, Workspace, Person ihres Teams, eigene Person.
    e.append(
        "U",
        "IssueLabel",
        vec!["org".into(), "orgpub".into()],
        json!({ "tag": "ws-label" }),
    )
    .await;
    e.append(
        "U",
        "Organization",
        vec!["org".into(), "orgpub".into()],
        json!({ "tag": "workspace" }),
    )
    .await;
    e.append(
        "U",
        "User",
        vec![
            "org".into(),
            g("user", d.bob.0),
            g("tmem", d.eng.0),
            g("tmem", d.web.0),
        ],
        json!({ "tag": "web-person" }),
    )
    .await;
    e.append(
        "U",
        "User",
        vec!["org".into(), g("user", d.carol.0), g("tmem", d.web.0)],
        json!({ "tag": "self" }),
    )
    .await;

    let mut got: Vec<Value> = vec![];
    while let Some(m) = next_sync(&mut carol, QUIET).await {
        got.extend(m["actions"].as_array().unwrap().iter().cloned());
    }
    let tags: Vec<&str> = got
        .iter()
        .map(|a| a["data"]["tag"].as_str().unwrap())
        .collect();
    assert_eq!(
        tags,
        ["ws-label", "workspace", "web-person", "self"],
        "{tags:?}"
    );
    // Fremde Team-/Mitgliedschafts-IDs stehen auch nicht in den Gruppen der gelieferten Aktionen.
    let wire = serde_json::to_string(&got).unwrap();
    assert!(
        !wire.contains(&d.eng.0.to_string()),
        "ENG-ID im Datenstrom: {wire}"
    );
    assert!(!wire.contains(&d.des.0.to_string()));
    assert!(!wire.contains(&d.alice.0.to_string()));
    // Der Nicht-Gast Bob erhält dagegen alles Sichtbare (`org`, Team).
    let mut bob_tags = vec![];
    while let Some(m) = next_sync(&mut bob, QUIET).await {
        bob_tags.extend(model_ids(&m));
    }
    for want in [
        "eng-issue",
        "eng-label",
        "initiative",
        "eng-only-person",
        "ws-label",
    ] {
        assert!(bob_tags.contains(&want.to_owned()), "{want}: {bob_tags:?}");
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn guest_gets_a_shared_issue_live_and_loses_it_on_unshare(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    // Issue in ENG: Carol (Gast, nur WEB) sieht es nicht.
    let issue = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
         VALUES ($1,$2,$3,200,'Geteiltes Issue',$4,$5)",
    )
    .bind(issue)
    .bind(d.org.0)
    .bind(d.eng.0)
    .bind(lineal_testkit::seed::identity::seed_workflow_state_ids(1)[1].0)
    .bind(d.bob.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let other = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
         VALUES ($1,$2,$3,201,'Nicht geteilt',$4,$5)",
    )
    .bind(other)
    .bind(d.org.0)
    .bind(d.eng.0)
    .bind(lineal_testkit::seed::identity::seed_workflow_state_ids(1)[1].0)
    .bind(d.bob.0)
    .execute(&e.pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb",
    )
    .execute(&e.pool)
    .await
    .unwrap();
    let worker = lineal_jobs::Worker::with(
        e.app.clone(),
        vec![Arc::new(lineal_jobs::consumers::sync::SyncConsumer)],
        vec![],
        lineal_jobs::WorkerConfig::default(),
    );
    let mut carol = e.connect("carol", None).await;
    next_json(&mut carol, WAIT).await.unwrap();

    // Ein Update am nicht geteilten Issue: Carol bekommt nichts.
    let bob = lineal_testkit::ctx_for(lineal_testkit::SeedUser::Alice);
    e.app
        .share_issue(&bob, &other.to_string(), "carol@acme.example")
        .await
        .unwrap();
    e.app
        .unshare_issue(&bob, &other.to_string(), "carol@acme.example")
        .await
        .unwrap();
    // Bob teilt `issue` mit Carol.
    let shared = e
        .app
        .share_issue(&bob, &issue.to_string(), "carol@acme.example")
        .await
        .unwrap();
    assert!(shared.shared);
    // Weder Erstellen noch Teilen erzeugen hier Sync-Aktionen, bis der Worker die Outbox verteilt.
    while worker.tick().await.unwrap().deliveries > 0 {}

    let mut got: Vec<Value> = vec![];
    while let Some(m) = next_sync(&mut carol, QUIET).await {
        got.extend(m["actions"].as_array().unwrap().iter().cloned());
    }
    let for_issue: Vec<&Value> = got
        .iter()
        .filter(|a| a["modelName"] == "Issue" && a["modelId"] == issue.to_string())
        .collect();
    assert!(
        !for_issue.is_empty(),
        "geteiltes Issue kommt live an: {got:?}"
    );
    let share = for_issue.last().unwrap();
    assert_eq!(share["data"]["title"], "Geteiltes Issue");
    // Nur die eigene Gruppe, nie die Team-ID von ENG.
    assert_eq!(share["syncGroups"], json!([g("user", d.carol.0)]));
    // Das andere Issue wurde geteilt und wieder entzogen, bevor der Worker die Events verteilte: der
    // Strom kennt nur den Endzustand (kein Zugriff), also erfährt Carol nichts darüber.
    assert!(
        !got.iter().any(|a| a["modelId"] == other.to_string()),
        "{got:?}"
    );

    // Entzug: Carol erhält `D` für das Issue.
    e.app
        .unshare_issue(&bob, &issue.to_string(), "carol@acme.example")
        .await
        .unwrap();
    while worker.tick().await.unwrap().deliveries > 0 {}
    let mut after: Vec<Value> = vec![];
    while let Some(m) = next_sync(&mut carol, QUIET).await {
        after.extend(m["actions"].as_array().unwrap().iter().cloned());
    }
    let last = after
        .iter()
        .rfind(|a| a["modelId"] == issue.to_string())
        .expect("Entzug wird zugestellt");
    assert_eq!(last["action"], "D");
    assert_eq!(last["syncGroups"], json!([g("user", d.carol.0)]));

    // Spätere Änderungen am Issue erreichen Carol nicht mehr.
    sqlx::query("UPDATE issue SET title = 'Neuer Titel' WHERE id = $1")
        .bind(issue)
        .execute(&e.pool)
        .await
        .unwrap();
    // (ein IssueUpdated-Event über den Use-Case-Weg: Share/Unshare am anderen Issue reicht nicht,
    // daher direkt ins Log mit den Gruppen, die `sync_model` jetzt liefern würde)
    let mut tx = e.app.store.begin(d.org).await.unwrap();
    let state = tx
        .outbox()
        .sync_model("Issue", issue)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.groups,
        vec![g("team", d.eng.0)],
        "keine Freigabe mehr"
    );
}

/// `GET /sync/bootstrap` als `who` (Session-Cookie); liefert Status und Zeilen.
async fn bootstrap(e: &Env, who: Option<&str>, query: &str) -> (StatusCode, Vec<String>) {
    use http_body_util::BodyExt;
    let router = router_with(e.app.clone(), fast());
    let mut req = Request::get(format!("/sync/bootstrap{query}"));
    if let Some(who) = who {
        req = req.header("cookie", format!("lineal_session={}", token(who)));
    }
    let res = router
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (status, text.lines().map(str::to_owned).collect())
}

/// Objekte (ohne Metazeile) und Metadaten einer Antwort.
fn split(lines: &[String]) -> (Vec<Value>, Value) {
    let (meta, rest) = lines.split_last().expect("Metazeile");
    let meta = serde_json::from_str(meta.strip_prefix("_metadata_=").expect("Metazeile")).unwrap();
    (
        rest.iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
        meta,
    )
}

/// Lädt alle Pakete mit `limit` und liefert alle Objekte plus die `lastSyncId`.
async fn bootstrap_all(e: &Env, who: &str, first: &str) -> (Vec<Value>, i64, usize) {
    let (mut objs, mut pages, mut last_id) = (vec![], 0, None);
    let mut query = first.to_owned();
    loop {
        let (status, lines) = bootstrap(e, Some(who), &query).await;
        assert_eq!(status, StatusCode::OK);
        let (o, meta) = split(&lines);
        objs.extend(o);
        pages += 1;
        let id = meta["lastSyncId"].as_i64().unwrap();
        assert_eq!(
            *last_id.get_or_insert(id),
            id,
            "lastSyncId bleibt über alle Pakete gleich"
        );
        match meta["nextCursor"].as_str() {
            Some(c) => query = format!("?cursor={}", c.replace(';', "%3B").replace(',', "%2C")),
            None => return (objs, last_id.unwrap(), pages),
        }
        assert!(pages < 200);
    }
}

fn class_ids(objs: &[Value], class: &str) -> Vec<String> {
    objs.iter()
        .filter(|o| o["__class"] == class)
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_rejects_bad_requests_and_anonymous_callers(pool: PgPool) {
    let e = env(pool).await;
    assert_eq!(bootstrap(&e, None, "").await.0, StatusCode::UNAUTHORIZED);
    for bad in [
        "?modelNames=Comment",
        "?modelNames=Issue,Issue",
        "?modelNames=",
        "?limit=0",
        "?limit=100000",
        "?cursor=nonsense",
        "?cursor=1%3B9%3B-%3BUser",
    ] {
        assert_eq!(
            bootstrap(&e, Some("bob"), bad).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    let (status, lines) = bootstrap(&e, Some("bob"), "?modelNames=Team&limit=5").await;
    assert_eq!(status, StatusCode::OK);
    assert!(lines.last().unwrap().starts_with("_metadata_="));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_never_leaks_foreign_teams_to_a_guest(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    // ENG-Issue, DES-Issue (privat), ENG-Dokument: Carol sieht keines davon.
    for (team, slot, number) in [(d.eng, 1_u128, 210), (d.des, 3, 211)] {
        sqlx::query(
            "INSERT INTO issue (organization_id, team_id, number, title, state_id, creator_id) \
             VALUES ($1,$2,$3,'Geheim',$4,$5)",
        )
        .bind(d.org.0)
        .bind(team.0)
        .bind(number)
        .bind(lineal_testkit::seed::identity::seed_workflow_state_ids(slot)[1].0)
        .bind(d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO document (organization_id, title, team_id, creator_id) VALUES ($1,'ENG-Doc',$2,$3)")
        .bind(d.org.0)
        .bind(d.eng.0)
        .bind(d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();

    let all = "?modelNames=Organization,Team,User,WorkflowState,IssueLabel,Cycle,Project,ProjectMilestone,Initiative,CustomView,Issue,Template,Document&limit=50";
    let (carol, _, _) = bootstrap_all(&e, "carol", all).await;
    let (bob, _, _) = bootstrap_all(&e, "bob", all).await;

    // Teams: nur WEB; Bob sieht alle drei.
    assert_eq!(class_ids(&carol, "Team"), [d.web.0.to_string()]);
    assert_eq!(class_ids(&bob, "Team").len(), 3);
    // Keine ENG-/DES-Spuren irgendwo im Gast-Bootstrap (IDs, Namen, Gruppen).
    let wire = serde_json::to_string(&carol).unwrap();
    for needle in [
        d.eng.0.to_string(),
        d.des.0.to_string(),
        "Geheim".into(),
        "ENG-Doc".into(),
    ] {
        assert!(!wire.contains(&needle), "{needle} im Gast-Bootstrap");
    }
    assert!(class_ids(&carol, "Issue").is_empty());
    assert!(class_ids(&carol, "Initiative").is_empty());
    assert!(class_ids(&carol, "Document").is_empty());
    // Bob sieht ENG-Issues und das Dokument.
    assert_eq!(class_ids(&bob, "Issue").len(), 2);
    assert_eq!(class_ids(&bob, "Document").len(), 1);

    // Personen: nur Mitglieder von WEB (und sie selbst), mit eigener Gruppe statt Team-Hinweisen.
    let allowed: Vec<String> = sqlx::query_scalar(
        "SELECT user_id::text FROM team_membership WHERE team_id = $1 AND archived_at IS NULL",
    )
    .bind(d.web.0)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    for id in class_ids(&carol, "User") {
        assert!(
            allowed.contains(&id) || id == d.carol.0.to_string(),
            "fremde Person {id} im Gast-Bootstrap"
        );
    }
    assert!(class_ids(&carol, "User").contains(&d.carol.0.to_string()));
    assert!(class_ids(&bob, "User").len() >= 3);
    // Die gelieferten Gruppen enthalten nur Gruppen des Empfängers.
    for o in &carol {
        for grp in o["syncGroups"].as_array().unwrap() {
            let grp = grp.as_str().unwrap();
            assert!(
                [
                    format!("team:{}", d.web.0),
                    format!("tmem:{}", d.web.0),
                    format!("user:{}", d.carol.0),
                    "orgpub".to_owned(),
                ]
                .contains(&grp.to_owned()),
                "{grp}"
            );
        }
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_includes_a_shared_issue_only_after_the_share_and_pages_are_consistent(
    pool: PgPool,
) {
    let e = env(pool).await;
    let d = e.d;
    let issue = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
         VALUES ($1,$2,$3,220,'Geteilt',$4,$5)",
    )
    .bind(issue)
    .bind(d.org.0)
    .bind(d.eng.0)
    .bind(lineal_testkit::seed::identity::seed_workflow_state_ids(1)[1].0)
    .bind(d.bob.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let first = "?modelNames=Issue,Team,User&limit=2";
    let (before, _, _) = bootstrap_all(&e, "carol", first).await;
    assert!(!class_ids(&before, "Issue").contains(&issue.to_string()));

    sqlx::query("INSERT INTO issue_share (organization_id, issue_id, user_id, shared_by_id) VALUES ($1,$2,$3,$4)")
        .bind(d.org.0)
        .bind(issue)
        .bind(d.carol.0)
        .bind(d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let (after, head, pages) = bootstrap_all(&e, "carol", first).await;
    assert_eq!(class_ids(&after, "Issue"), [issue.to_string()]);
    assert!(
        pages >= 2,
        "kleine Pakete werden über den Cursor fortgesetzt"
    );
    // Paketweise = in einem Zug (gleiche Objektmenge).
    let (one, head_one, one_pages) =
        bootstrap_all(&e, "carol", "?modelNames=Issue,Team,User&limit=2000").await;
    assert_eq!(one_pages, 1);
    assert_eq!(head, head_one);
    let mut a: Vec<String> = after
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    let mut b: Vec<String> = one
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    // Das geteilte Issue trägt nur die eigene Gruppe von Carol (keine ENG-ID).
    let obj = after.iter().find(|o| o["id"] == issue.to_string()).unwrap();
    assert_eq!(obj["syncGroups"], json!([format!("user:{}", d.carol.0)]));
    assert_eq!(obj["__class"], "Issue");
}

// --- Sicherheitsprüfung: Bootstrap je Modellklasse -------------------------------------------

/// Legt je Klasse ein Objekt im privaten Team DES an (plus ein archiviertes/Papierkorb-Objekt in ENG)
/// und liefert `(Klasse, id)`.
async fn des_fixtures(e: &Env) -> Vec<(&'static str, String)> {
    let d = e.d;
    let mut out = vec![];
    let one = |sql: String| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>(&sql)
                .fetch_one(&pool)
                .await
                .unwrap()
                .to_string()
        }
    };
    let (org, des, bob) = (d.org.0, d.des.0, d.bob.0);
    out.push((
        "IssueLabel",
        one(format!("INSERT INTO issue_label (organization_id, team_id, name, color) VALUES ('{org}','{des}','Des-Label','#112233') RETURNING id")).await,
    ));
    out.push((
        "Cycle",
        one(format!("INSERT INTO cycle (organization_id, team_id, number, starts_at, ends_at) VALUES ('{org}','{des}',90, now(), now() + interval '7 days') RETURNING id")).await,
    ));
    out.push((
        "Template",
        one(format!("INSERT INTO template (organization_id, type, name, team_id) VALUES ('{org}','issue','Des-Template','{des}') RETURNING id")).await,
    ));
    out.push((
        "CustomView",
        one(format!("INSERT INTO custom_view (organization_id, name, model_name, filter_data, owner_id, creator_id, shared, team_id) VALUES ('{org}','Des-View','Issue','{{}}','{bob}','{bob}',true,'{des}') RETURNING id")).await,
    ));
    out.push((
        "Document",
        one(format!("INSERT INTO document (organization_id, title, team_id, creator_id) VALUES ('{org}','Des-Doc','{des}','{bob}') RETURNING id")).await,
    ));
    out.push((
        "Issue",
        one(format!(
            "INSERT INTO issue (organization_id, team_id, number, title, state_id, creator_id) VALUES ('{org}','{des}',300,'Des-Issue','{}','{bob}') RETURNING id",
            lineal_testkit::seed::identity::seed_workflow_state_ids(3)[1].0
        )).await,
    ));
    let status: uuid::Uuid = sqlx::query_scalar("SELECT id FROM project_status LIMIT 1")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let project = one(format!("INSERT INTO project (organization_id, name, lead_team_id, number, status_id, creator_id) VALUES ('{org}','Des-Project','{des}',300,'{status}','{bob}') RETURNING id")).await;
    sqlx::query(
        "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2::uuid,$3)",
    )
    .bind(org)
    .bind(&project)
    .bind(des)
    .execute(&e.pool)
    .await
    .unwrap();
    let milestone = one(format!("INSERT INTO project_milestone (organization_id, project_id, name) VALUES ('{org}','{project}','Des-Milestone') RETURNING id")).await;
    out.push(("Project", project));
    out.push(("ProjectMilestone", milestone));
    out.push(("Team", des.to_string()));
    // Archiviertes ENG-Objekt und Issue im Papierkorb gehören nie zum Bootstrap.
    let trashed = one(format!(
        "INSERT INTO issue (organization_id, team_id, number, title, state_id, creator_id, trashed, trashed_at) VALUES ('{org}','{}',301,'Papierkorb','{}','{bob}', true, now()) RETURNING id",
        d.eng.0,
        lineal_testkit::seed::identity::seed_workflow_state_ids(1)[1].0
    ))
    .await;
    out.push(("TrashedIssue", trashed));
    out
}

const ALL_MODELS: &str = "?modelNames=Organization,Team,User,WorkflowState,IssueLabel,Cycle,Project,ProjectMilestone,Initiative,CustomView,Issue,Template,Document&limit=100";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_private_team_objects_reach_only_members_in_every_class(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    let fixtures = des_fixtures(&e).await;
    // DES-Gruppe des Teams selbst verwendet `team:<DES>`; Alice (kein DES) und Carol (Gast, nur WEB)
    // dürfen keine dieser Objekte sehen, Bob (Mitglied) schon (Kontrolle gegen Leerlauf).
    let (bob, _, _) = bootstrap_all(&e, "bob", ALL_MODELS).await;
    for (who, expect_all) in [("alice", false), ("carol", false), ("bob", true)] {
        let (objs, _, _) = if who == "bob" {
            (bob.clone(), 0, 0)
        } else {
            bootstrap_all(&e, who, ALL_MODELS).await
        };
        let wire = serde_json::to_string(&objs).unwrap();
        for (class, id) in &fixtures {
            let present = wire.contains(id.as_str());
            if *class == "TrashedIssue" {
                assert!(!present, "{who}: Papierkorb-Issue geliefert");
            } else {
                assert_eq!(present, expect_all, "{who}: {class} {id}");
            }
        }
        // Keine E-Mail-Adressen und keine Namen privater Objekte in den Antworten.
        assert!(!wire.contains("@acme.example"), "{who}: E-Mail geliefert");
        if !expect_all {
            for name in [
                "Des-Label",
                "Des-Template",
                "Des-View",
                "Des-Doc",
                "Des-Issue",
                "Des-Project",
                "Des-Milestone",
            ] {
                assert!(!wire.contains(name), "{who}: {name}");
            }
        }
        // Nur bekannte Klassen.
        for o in &objs {
            let c = o["__class"].as_str().unwrap();
            assert!(ALL_MODELS.contains(c), "{c}");
        }
    }
    let _ = d;
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_private_views_and_admin_only_data_are_never_delivered(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    // Private View von Bob und Workspace-View: nur der Besitzer bzw. Nicht-Gäste.
    for (name, shared) in [("Bobs-privat", false), ("WS-geteilt", true)] {
        sqlx::query("INSERT INTO custom_view (organization_id, name, model_name, filter_data, owner_id, creator_id, shared) VALUES ($1,$2,'Issue','{}'::jsonb,$3,$3,$4)")
            .bind(d.org.0)
            .bind(name)
            .bind(d.bob.0)
            .bind(shared)
            .execute(&e.pool)
            .await
            .unwrap();
    }
    let (alice, _, _) = bootstrap_all(&e, "alice", ALL_MODELS).await;
    let (carol, _, _) = bootstrap_all(&e, "carol", ALL_MODELS).await;
    let (bob, _, _) = bootstrap_all(&e, "bob", ALL_MODELS).await;
    let names = |o: &[Value]| -> Vec<String> {
        o.iter()
            .filter(|x| x["__class"] == "CustomView")
            .map(|x| x["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(names(&bob).contains(&"Bobs-privat".to_owned()));
    assert!(!names(&alice).contains(&"Bobs-privat".to_owned()));
    assert!(!names(&carol).contains(&"Bobs-privat".to_owned()));
    assert!(names(&alice).contains(&"WS-geteilt".to_owned()));
    assert!(
        !names(&carol).contains(&"WS-geteilt".to_owned()),
        "Gast: kein Workspace-View"
    );

    // Admin-Klassen (Einladungen, Keys, Webhooks, Audit, Sicherheit, Kommentare) sind nicht abrufbar.
    for model in [
        "Invitation",
        "ApiKey",
        "Webhook",
        "AuditLog",
        "OrganizationSecurity",
        "UserSession",
        "Comment",
        "Notification",
    ] {
        for who in ["alice", "carol"] {
            assert_eq!(
                bootstrap(&e, Some(who), &format!("?modelNames={model}"))
                    .await
                    .0,
                StatusCode::BAD_REQUEST,
                "{who}: {model}"
            );
        }
    }
    // Der Workspace enthält nur Id und Name.
    let org = alice
        .iter()
        .find(|o| o["__class"] == "Organization")
        .unwrap();
    let mut keys: Vec<&String> = org.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["__class", "id", "name", "syncGroups"]);
    // Personen: nur Id und Namen, nie E-Mail/Rolle.
    for o in alice
        .iter()
        .chain(carol.iter())
        .filter(|o| o["__class"] == "User")
    {
        let mut keys: Vec<&String> = o.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "__class",
                "displayName",
                "id",
                "name",
                "syncGroups",
                "updatedAt"
            ]
        );
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_manipulated_cursors_cannot_skip_the_filter_or_fake_the_head(pool: PgPool) {
    let e = env(pool).await;
    let d = e.d;
    let fixtures = des_fixtures(&e).await;
    let issue_id = fixtures.iter().find(|f| f.0 == "Issue").unwrap().1.clone();
    // Cursor mitten in der Issue-Klasse, ab Nil-UUID, mit absurd hohem Stand.
    let cursor = "999999999;1;00000000-0000-0000-0000-000000000000;Team,Issue,Document";
    let q = format!("?cursor={}", cursor.replace(';', "%3B").replace(',', "%2C"));
    for who in ["alice", "carol"] {
        let (status, lines) = bootstrap(&e, Some(who), &q).await;
        assert_eq!(status, StatusCode::OK);
        let (objs, meta) = split(&lines);
        assert!(
            !serde_json::to_string(&objs).unwrap().contains(&issue_id),
            "{who}"
        );
        assert!(
            objs.iter().all(|o| o["__class"] != "Team"),
            "{who}: Cursor überspringt nie die Rechteprüfung, Klassen vor idx entfallen"
        );
        // Der gemeldete Stand ist nie größer als der echte Kopf des Logs.
        let mut tx = e.app.store.begin(d.org).await.unwrap();
        let head = tx.outbox().sync_head().await.unwrap().head;
        assert!(meta["lastSyncId"].as_i64().unwrap() <= head);
    }
    // Fremde Organisation: der Cursor trägt keine Organisation, jede Abfrage nutzt die des Tokens.
    let other_org = Uuid::now_v7();
    let bad = format!("?cursor=1%3B0%3B{other_org}%3BIssue");
    let (status, lines) = bootstrap(&e, Some("carol"), &bad).await;
    assert_eq!(status, StatusCode::OK);
    let (objs, _) = split(&lines);
    assert!(objs.is_empty(), "Carol sieht keine Issues");
}
