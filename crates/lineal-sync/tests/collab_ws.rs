#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP36: Echtzeit-Mitbearbeitung gegen echte Datenbank und echten Socket (Seed `acme`):
//! Konvergenz, Persistenz, Rechte, Grenzen, API-Schreibvorgänge in offenen Räumen.

use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use lineal_app::collab::{crdt, pm};
use lineal_app::ports::Store;
use lineal_app::ports::repos::auth::NewSession;
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::rules::patch::PatchOp;
use lineal_sync::collab::CollabConfig;
use lineal_sync::{SyncConfig, router_with};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
use yrs::sync::{Message as YMessage, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update, XmlFragment};

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

fn collab_cfg() -> CollabConfig {
    CollabConfig {
        persist_quiet: Duration::from_millis(150),
        persist_max: Duration::from_millis(600),
        poll_interval: Duration::from_millis(50),
        idle_unload: Duration::from_millis(400),
        revalidate_interval: Duration::from_millis(300),
        recheck_throttle: Duration::from_millis(100),
        ..CollabConfig::default()
    }
}

async fn env_with(pool: PgPool, collab: CollabConfig) -> Env {
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
    let cfg = SyncConfig {
        poll_interval: Duration::from_millis(40),
        collab,
        ..SyncConfig::default()
    };
    let router = router_with(app.clone(), cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Env { app, pool, addr, d }
}

async fn env(pool: PgPool) -> Env {
    env_with(pool, collab_cfg()).await
}

fn user(u: SeedUser) -> RequestCtx {
    ctx_for(u)
}

impl Env {
    async fn doc(&self, who: SeedUser, team: &str, title: &str, content: &str) -> Uuid {
        self.app
            .save_document(
                &user(who),
                SaveDocumentInput {
                    title: Some(title.into()),
                    content: Some(content.into()),
                    parent: ParentRefs {
                        team: Some(team.into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .doc
            .id
            .0
    }

    async fn try_connect(
        &self,
        who: Option<&str>,
        path: &str,
        headers: &[(&str, &str)],
    ) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}{path}", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(raw) = who.and_then(|w| w.strip_prefix("key:")) {
            req.headers_mut()
                .insert("authorization", format!("Bearer {raw}").parse().unwrap());
        } else if let Some(who) = who {
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

    async fn join(&self, who: &str, doc: Uuid) -> Client {
        let ws = self
            .try_connect(Some(who), &format!("/collab/document/{doc}"), &[])
            .await
            .unwrap();
        Client::start(ws).await
    }

    async fn status(&self, who: Option<&str>, path: &str) -> u16 {
        match self.try_connect(who, path, &[]).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => res.status().as_u16(),
            Ok(_) => 101,
            Err(e) => panic!("{e}"),
        }
    }

    async fn db_doc(&self, id: Uuid) -> (String, Option<String>, i32, i64) {
        sqlx::query_as(
            "SELECT c.markdown, c.content_state, c.version, \
             (SELECT count(*) FROM document_content_history h WHERE h.document_content_id = c.id) \
             FROM document_content c WHERE c.owner_type = 'document' AND c.owner_id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

/// Ein simulierter Browser-Client (Yjs-Dokument + y-protocols).
struct Client {
    ws: Ws,
    doc: Doc,
    hello: Value,
    awareness: Vec<(u64, String)>,
    closed: Option<u16>,
}

impl Client {
    async fn start(mut ws: Ws) -> Self {
        // Hello (Text), dann Schritt 1 des Servers.
        let hello = loop {
            match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => break serde_json::from_str::<Value>(&t).unwrap(),
                Ok(Some(Ok(_))) => {}
                other => panic!("no hello: {other:?}"),
            }
        };
        let doc = crdt::new_doc();
        let _ = doc.get_or_insert_xml_fragment(pm::FRAGMENT);
        let mut c = Self {
            ws,
            doc,
            hello,
            awareness: vec![],
            closed: None,
        };
        let sv = c.doc.transact().state_vector();
        c.send(&YMessage::Sync(SyncMessage::SyncStep1(sv))).await;
        c.pump(Duration::from_millis(300)).await;
        c
    }

    async fn send(&mut self, m: &YMessage) {
        let _ = self.ws.send(Message::Binary(m.encode_v1().into())).await;
    }

    async fn send_raw(&mut self, bytes: Vec<u8>) {
        let _ = self.ws.send(Message::Binary(bytes.into())).await;
    }

    /// Verarbeitet eingehende Frames für `dur`.
    async fn pump(&mut self, dur: Duration) {
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Ok(next) = tokio::time::timeout(left, self.ws.next()).await else {
                return;
            };
            match next {
                Some(Ok(Message::Binary(b))) => {
                    let Ok(m) = YMessage::decode_v1(&b) else {
                        continue;
                    };
                    match m {
                        YMessage::Sync(SyncMessage::SyncStep1(sv)) => {
                            let u = self.doc.transact().encode_state_as_update_v1(&sv);
                            self.send(&YMessage::Sync(SyncMessage::SyncStep2(u))).await;
                        }
                        YMessage::Sync(SyncMessage::SyncStep2(u) | SyncMessage::Update(u)) => {
                            self.doc
                                .transact_mut()
                                .apply_update(Update::decode_v1(&u).unwrap())
                                .unwrap();
                        }
                        YMessage::Awareness(a) => {
                            for (id, e) in a.clients {
                                self.awareness.push((id, e.json.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(f))) => {
                    self.closed = Some(f.map_or(1005, |f| f.code.into()));
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    self.closed.get_or_insert(0);
                    return;
                }
            }
        }
    }

    fn md(&self) -> String {
        crdt::markdown_of(&self.doc)
    }

    /// Lokale Änderung und Update an den Server.
    async fn edit(&mut self, f: impl FnOnce(&Doc)) {
        let before = self.doc.transact().state_vector();
        f(&self.doc);
        let u = self.doc.transact().encode_state_as_update_v1(&before);
        self.send(&YMessage::Sync(SyncMessage::Update(u))).await;
    }

    async fn add_paragraph(&mut self, text: &str) {
        let blocks = pm::parse_markdown(text);
        self.edit(|doc| {
            let frag = doc.get_or_insert_xml_fragment(pm::FRAGMENT);
            let mut txn = doc.transact_mut();
            pm::insert_nodes(&frag, &mut txn, 0, &blocks);
        })
        .await;
    }
}

async fn pump_both(a: &mut Client, b: &mut Client, ms: u64) {
    tokio::join!(
        a.pump(Duration::from_millis(ms)),
        b.pump(Duration::from_millis(ms))
    );
}

// --- Tests ------------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn two_clients_converge_on_concurrent_inserts_and_persist(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "Anfang").await;
    let (v0, h0) = {
        let r = e.db_doc(doc).await;
        (r.2, r.3)
    };
    let mut alice = e.join("alice", doc).await;
    let mut bob = e.join("bob", doc).await;
    assert_eq!(alice.hello["canWrite"], true);
    assert_eq!(alice.md(), "Anfang");
    assert_eq!(bob.md(), "Anfang");
    assert_eq!(alice.hello["epoch"], bob.hello["epoch"]);

    // Gleichzeitige Einfügungen an derselben Stelle (vorn).
    alice.add_paragraph("Text von Alice").await;
    bob.add_paragraph("Text von Bob").await;
    pump_both(&mut alice, &mut bob, 600).await;
    assert_eq!(alice.md(), bob.md());
    assert!(alice.md().contains("Text von Alice"), "{}", alice.md());
    assert!(alice.md().contains("Text von Bob"), "{}", alice.md());
    assert!(alice.md().ends_with("Anfang"));

    // Persistenz nach dem Debounce: Markdown, Zustand, Version, Snapshot.
    let mut persisted = false;
    for _ in 0..60 {
        let r = e.db_doc(doc).await;
        if r.0.contains("Text von Alice") && r.0.contains("Text von Bob") {
            assert!(r.1.is_some(), "contentState gespeichert");
            assert!(r.2 > v0, "Version erhöht");
            assert!(r.3 >= h0);
            persisted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(persisted, "Raum wurde nicht gespeichert");

    // Alle gehen, Raum wird entladen; neues Öffnen liefert denselben Stand und dieselbe Epoche.
    let epoch = alice.hello["epoch"].clone();
    drop(alice);
    drop(bob);
    tokio::time::sleep(Duration::from_millis(900)).await;
    let again = e.join("bob", doc).await;
    assert!(again.md().contains("Text von Bob"));
    assert_eq!(again.hello["epoch"], epoch);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_update_during_open_session_lands_in_room_without_loss(pool: PgPool) {
    let e = env(pool).await;
    let doc = e
        .doc(SeedUser::Alice, "ENG", "Spec", "eins\n\nzwei\n\ndrei")
        .await;
    let mut alice = e.join("alice", doc).await;
    // Alice tippt (noch nicht gespeichert) ...
    alice.add_paragraph("lokal").await;
    // ... Bob schreibt per API (Patch) in dasselbe Dokument.
    e.app
        .save_document(
            &user(SeedUser::Bob),
            SaveDocumentInput {
                id: Some(doc.to_string()),
                patch: Some(vec![PatchOp::Append {
                    text: "\n\nvia API".into(),
                }]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for _ in 0..40 {
        alice.pump(Duration::from_millis(100)).await;
        if alice.md().contains("via API") {
            break;
        }
    }
    assert!(alice.md().contains("via API"), "{}", alice.md());
    assert!(
        alice.md().contains("lokal"),
        "lokale Änderung bleibt: {}",
        alice.md()
    );
    assert!(alice.md().contains("zwei"));
    // Beides wird zusammengeführt gespeichert.
    wait_for_db(&e, doc, |m| m.contains("via API") && m.contains("lokal")).await;
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_patch_over_multibyte_text_updates_open_room(pool: PgPool) {
    let e = env(pool).await;
    let doc = e
        .doc(SeedUser::Alice, "ENG", "Spec", "Hallo 😀 Welt ä 日本")
        .await;
    let mut alice = e.join("alice", doc).await;
    e.app
        .save_document(
            &user(SeedUser::Bob),
            SaveDocumentInput {
                id: Some(doc.to_string()),
                patch: Some(vec![PatchOp::Replace {
                    old_string: "😀".into(),
                    new_string: "👨‍👩‍👧 und é".into(),
                    replace_all: None,
                }]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for _ in 0..40 {
        alice.pump(Duration::from_millis(100)).await;
        if alice.md().contains("und é") {
            break;
        }
    }
    assert_eq!(alice.md(), "Hallo 👨‍👩‍👧 und é Welt ä 日本");
}

/// B2.01: Tipptext im offenen Raum (noch nicht gespeichert) darf durch ein API/MCP-Schreiben
/// binnen des Debounce-Fensters nicht verloren gehen - auch beim Schreiben im selben Absatz.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_patch_within_debounce_window_keeps_unsaved_typing(pool: PgPool) {
    let cfg = CollabConfig {
        persist_quiet: Duration::from_millis(1500),
        persist_max: Duration::from_millis(6000),
        ..collab_cfg()
    };
    let e = env_with(pool, cfg).await;
    // Mehrere Inhalte: u. a. solche, deren Markdown beim Parsen/Ausgeben nicht stabil ist.
    let contents = [
        "Anfang Text\n\nZweiter Absatz",
        "* Anfang Text\n* eins\n\nZweiter Absatz",
        "Anfang Text  \nmit Umbruch\n\nZweiter Absatz",
        "Anfang Text\n\n\n\nZweiter Absatz",
    ];
    for (delay_ms, content) in [100u64, 400, 900, 1400]
        .into_iter()
        .zip(contents.into_iter().cycle())
    {
        let doc = e.doc(SeedUser::Alice, "ENG", "Spec", content).await;
        let mut alice = e.join("alice", doc).await;
        alice
            .edit(|d| {
                let md = crdt::markdown_of(d);
                let typed = md.replacen("Anfang Text", "Anfang Text TIPP", 1);
                crdt::apply_markdown(d, &typed);
            })
            .await;
        alice.pump(Duration::from_millis(delay_ms)).await;
        e.app
            .save_document(
                &user(SeedUser::Bob),
                SaveDocumentInput {
                    id: Some(doc.to_string()),
                    patch: Some(vec![PatchOp::Replace {
                        old_string: "Zweiter".into(),
                        new_string: "API-Zweiter".into(),
                        replace_all: None,
                    }]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for _ in 0..30 {
            alice.pump(Duration::from_millis(100)).await;
            if alice.md().contains("API-Zweiter") {
                break;
            }
        }
        assert!(alice.md().contains("API-Zweiter"), "{}", alice.md());
        assert!(
            alice.md().contains("TIPP"),
            "delay {delay_ms}: Raum verlor Tipptext: {}",
            alice.md()
        );
        wait_for_db(&e, doc, |m| m.contains("API-Zweiter") && m.contains("TIPP")).await;
        drop(alice);
    }
}

async fn wait_for_db(e: &Env, doc: Uuid, f: impl Fn(&str) -> bool) {
    for _ in 0..80 {
        if f(&e.db_doc(doc).await.0) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("db not updated: {}", e.db_doc(doc).await.0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_write_without_room_keeps_state_lineage(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "erst").await;
    let first = e.join("alice", doc).await;
    let epoch = first.hello["epoch"].clone();
    drop(first);
    tokio::time::sleep(Duration::from_millis(900)).await;
    // API-Schreibvorgang ohne offenen Raum: Zustand wird mitgeführt, Epoche bleibt.
    e.app
        .save_document(
            &user(SeedUser::Bob),
            SaveDocumentInput {
                id: Some(doc.to_string()),
                content: Some("erst\n\nzweit".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let again = e.join("alice", doc).await;
    assert_eq!(again.md(), "erst\n\nzweit");
    assert_eq!(again.hello["epoch"], epoch);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn stale_state_is_discarded_when_markdown_was_changed_behind_its_back(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "alt").await;
    let first = e.join("alice", doc).await;
    let epoch = first.hello["epoch"].clone();
    drop(first);
    tokio::time::sleep(Duration::from_millis(900)).await;
    // Schreiben am Use-Case vorbei (z. B. Import): Markdown ist nicht mehr der des Zustands.
    sqlx::query("UPDATE document_content SET markdown = 'neu' WHERE owner_id = $1")
        .bind(doc)
        .execute(&e.pool)
        .await
        .unwrap();
    let again = e.join("alice", doc).await;
    assert_eq!(again.md(), "neu");
    assert_ne!(again.hello["epoch"], epoch, "neue CRDT-Linie");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn read_only_user_cannot_write_and_is_disconnected(pool: PgPool) {
    let e = env(pool).await;
    // Reiner Leser: API-Key mit Scope `read` (Alice' Rechte, aber ohne Schreib-Scope).
    sqlx::query(
        "INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash, scopes) \
         VALUES ($1, $2, 'ro', 'lin_ro', $3, ARRAY['read'])",
    )
    .bind(e.d.org.0)
    .bind(e.d.alice.0)
    .bind(lineal_app::usecases::auth::hash_token("lin_readonly_key"))
    .execute(&e.pool)
    .await
    .unwrap();
    let web_doc = e.doc(SeedUser::Alice, "ENG", "Lesen", "Inhalt").await;
    let doc = web_doc;
    let ws = e
        .try_connect(
            Some("key:lin_readonly_key"),
            &format!("/collab/document/{doc}"),
            &[],
        )
        .await
        .unwrap();
    let mut carol = Client::start(ws).await;
    assert_eq!(carol.hello["canWrite"], false);
    assert_eq!(carol.md(), "Inhalt");
    carol.add_paragraph("Vandalismus").await;
    carol.pump(Duration::from_millis(500)).await;
    assert_eq!(carol.closed, Some(4403));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(e.db_doc(doc).await.0, "Inhalt");
    let bob = e.join("bob", doc).await;
    assert_eq!(bob.md(), "Inhalt");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn invisible_and_foreign_content_is_404_without_hint(pool: PgPool) {
    let e = env(pool).await;
    let eng = e.doc(SeedUser::Alice, "ENG", "Intern", "geheim").await;
    let des = e.doc(SeedUser::Bob, "DES", "Design", "privat").await;
    // Carol (Gast, nur WEB) sieht weder ENG- noch DES-Dokumente.
    for id in [eng, des] {
        let path = format!("/collab/document/{id}");
        assert_eq!(e.status(Some("carol"), &path).await, 404);
    }
    // Nicht vorhandene ID: identische Antwort.
    let missing = format!("/collab/document/{}", Uuid::now_v7());
    assert_eq!(e.status(Some("carol"), &missing).await, 404);
    // Unbekannte Inhaltsart und kaputte ID.
    assert_eq!(
        e.status(Some("bob"), &format!("/collab/issue/{eng}")).await,
        404
    );
    assert_eq!(
        e.status(Some("bob"), "/collab/document/nicht-uuid").await,
        400
    );
    // Ohne Anmeldung: 401.
    let eng_path = format!("/collab/document/{eng}");
    assert_eq!(e.status(None, &eng_path).await, 401);
    // Fremder Workspace: ein Nutzer eines anderen Workspace kennt die ID, sieht aber nichts.
    let beta = e
        .app
        .create_workspace(lineal_app::usecases::org::CreateWorkspaceInput {
            name: "Beta".into(),
            url_key: Some("beta".into()),
            owner_name: "Zoe Example".into(),
            owner_email: "zoe@beta.example".into(),
            owner_password: Some("correct horse battery".into()),
            credential_kind: None,
        })
        .await
        .unwrap();
    let mut tx = e.app.store.begin(beta.org.id).await.unwrap();
    tx.auth()
        .insert_session(&NewSession {
            id: Uuid::now_v7(),
            user_id: beta.owner.id,
            token_hash: lineal_app::usecases::auth::hash_token(&token("zoe")),
            csrf_token_hash: None,
            user_agent: None,
            ip_address: None,
            expires_at: Utc::now() + ChronoDuration::days(1),
            now: Utc::now(),
        })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(e.status(Some("zoe"), &eng_path).await, 404);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn foreign_origin_is_rejected(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    match e
        .try_connect(
            Some("alice"),
            &format!("/collab/document/{doc}"),
            &[("origin", "https://evil.example")],
        )
        .await
    {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => assert_eq!(res.status(), 403),
        other => panic!("expected 403, got {:?}", other.map(|_| ())),
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn token_kind_must_match_the_transport(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    let path = format!("/collab/document/{doc}");
    // Session-Token im Authorization-Header: abgelehnt.
    let session = token("alice");
    assert_eq!(
        match e
            .try_connect(
                None,
                &path,
                &[("authorization", &format!("Bearer {session}"))]
            )
            .await
        {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => res.status().as_u16(),
            _ => 101,
        },
        401
    );
    // API-Key im Session-Cookie: abgelehnt.
    sqlx::query(
        "INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash) \
         VALUES ($1, $2, 'rw', 'lin_rw', $3)",
    )
    .bind(e.d.org.0)
    .bind(e.d.alice.0)
    .bind(lineal_app::usecases::auth::hash_token("lin_key_in_cookie"))
    .execute(&e.pool)
    .await
    .unwrap();
    assert_eq!(
        match e
            .try_connect(
                None,
                &path,
                &[("cookie", "lineal_session=lin_key_in_cookie")]
            )
            .await
        {
            Err(tokio_tungstenite::tungstenite::Error::Http(res)) => res.status().as_u16(),
            _ => 101,
        },
        401
    );
    // Richtig transportiert: ok.
    assert_eq!(e.status(Some("key:lin_key_in_cookie"), &path).await, 101);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn revoked_api_key_is_disconnected_and_gets_no_further_updates(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    sqlx::query(
        "INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash) \
         VALUES ($1, $2, 'rw', 'lin_rw', $3)",
    )
    .bind(e.d.org.0)
    .bind(e.d.alice.0)
    .bind(lineal_app::usecases::auth::hash_token("lin_revoke_me"))
    .execute(&e.pool)
    .await
    .unwrap();
    let ws = e
        .try_connect(
            Some("key:lin_revoke_me"),
            &format!("/collab/document/{doc}"),
            &[],
        )
        .await
        .unwrap();
    let mut keyed = Client::start(ws).await;
    let mut bob = e.join("bob", doc).await;
    sqlx::query("UPDATE api_key SET revoked_at = now() WHERE key_hash = $1")
        .bind(lineal_app::usecases::auth::hash_token("lin_revoke_me"))
        .execute(&e.pool)
        .await
        .unwrap();
    keyed.pump(Duration::from_secs(3)).await;
    assert_eq!(keyed.closed, Some(4401));
    // Spätere Änderungen erreichen den getrennten Client nicht mehr.
    bob.add_paragraph("danach").await;
    keyed.pump(Duration::from_millis(500)).await;
    assert!(!keyed.md().contains("danach"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn broken_and_oversized_updates_close_the_connection(pool: PgPool) {
    let e = env_with(
        pool,
        CollabConfig {
            max_update_bytes: 4096,
            ..collab_cfg()
        },
    )
    .await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "stabil").await;
    let mut good = e.join("bob", doc).await;

    // Kaputte Bytes: Schließen mit 4400, kein Panic, Raum bleibt intakt.
    let mut c = e.join("alice", doc).await;
    c.send_raw(vec![0, 2, 5, 255, 255, 255, 255, 1, 2, 3]).await;
    c.pump(Duration::from_millis(400)).await;
    assert_eq!(c.closed, Some(4400));
    let mut c = e.join("alice", doc).await;
    c.send_raw(vec![99, 1, 2, 3]).await;
    c.pump(Duration::from_millis(400)).await;
    assert_eq!(c.closed, Some(4400));

    // Zu großes Update (über dem Frame-Limit trennt schon das Protokoll).
    let mut c = e.join("alice", doc).await;
    c.add_paragraph(&"x".repeat(8000)).await;
    c.pump(Duration::from_millis(500)).await;
    assert!(c.closed.is_some(), "{:?}", c.closed);

    good.pump(Duration::from_millis(300)).await;
    assert_eq!(good.md(), "stabil");
    assert_eq!(good.closed, None);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn deeply_nested_updates_are_rejected_and_rolled_back(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "stabil").await;
    let mut victim = e.join("bob", doc).await;
    let mut attacker = e.join("alice", doc).await;
    attacker
        .edit(|d| {
            let frag = d.get_or_insert_xml_fragment(pm::FRAGMENT);
            let mut txn = d.transact_mut();
            let mut cur = frag.insert(&mut txn, 0, yrs::XmlElementPrelim::empty("blockquote"));
            for _ in 0..10_000 {
                cur = cur.insert(&mut txn, 0, yrs::XmlElementPrelim::empty("blockquote"));
            }
        })
        .await;
    attacker.pump(Duration::from_millis(800)).await;
    assert!(
        matches!(attacker.closed, Some(1009) | Some(0) | Some(1005)),
        "{:?}",
        attacker.closed
    );
    victim.pump(Duration::from_millis(400)).await;
    assert_eq!(victim.md(), "stabil", "Update wurde nicht verteilt");
    assert_eq!(victim.closed, None);
    // Der Server läuft weiter, der Raum ist intakt.
    let again = e.join("alice", doc).await;
    assert_eq!(again.md(), "stabil");
    assert!(crdt::check_limits(&again.doc).is_ok());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn deeply_nested_markdown_over_the_api_does_not_crash_open_rooms(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "stabil").await;
    let mut alice = e.join("alice", doc).await;
    for unit in ["> ", "- ", "*", "1. "] {
        let md = format!("{}tief", unit.repeat(10_000));
        // Je nach Bereinigung angenommen oder abgelehnt – nie ein Absturz.
        let _ = e
            .app
            .save_document(
                &user(SeedUser::Bob),
                SaveDocumentInput {
                    id: Some(doc.to_string()),
                    content: Some(md),
                    ..Default::default()
                },
            )
            .await;
        alice.pump(Duration::from_millis(300)).await;
        assert!(crdt::check_limits(&alice.doc).is_ok());
    }
    assert_eq!(alice.closed, None);
}

fn aw(id: u64, clock: u32, json: String) -> YMessage {
    YMessage::Awareness(AwarenessUpdate {
        clients: [(
            id,
            AwarenessUpdateEntry {
                clock,
                json: json.into(),
            },
        )]
        .into(),
    })
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn awareness_is_rewritten_to_the_authenticated_identity(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    let mut alice = e.join("alice", doc).await;
    let mut bob = e.join("bob", doc).await;
    let alice_name = alice.hello["user"]["name"].as_str().unwrap().to_owned();
    // Alice behauptet, jemand anderes zu sein, und schmuggelt Zusatzfelder ein.
    let evil = json!({
        "user": { "name": "Bob the Admin", "color": "#000000", "id": "fake" },
        "cursor": { "anchor": { "type": null }, "head": { "type": null }, "extra": "x" },
        "payload": "<script>"
    })
    .to_string();
    alice.send(&aw(42, 1, evil)).await;
    bob.pump(Duration::from_millis(500)).await;
    let (_, json) = bob.awareness.iter().find(|(id, _)| *id == 42).unwrap();
    let v: Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["user"]["name"], alice_name.as_str());
    assert_ne!(v["user"]["color"], "#000000");
    assert_ne!(v["user"]["id"], "fake");
    assert!(v.get("payload").is_none());
    assert!(v["cursor"].get("extra").is_none());

    // Cursor `null` (Editor ohne Fokus) ist normal und wird nicht abgelehnt.
    alice
        .send(&aw(
            43,
            1,
            json!({"user": {"name": "x"}, "cursor": null}).to_string(),
        ))
        .await;
    alice.pump(Duration::from_millis(300)).await;
    assert_eq!(alice.closed, None);

    // Fremde Client-ID übernehmen: Verbindung wird getrennt.
    bob.send(&aw(42, 9, "{}".into())).await;
    bob.pump(Duration::from_millis(500)).await;
    assert_eq!(bob.closed, Some(4400));

    // Nur-Leser dürfen Awareness senden (Cursor), aber nur mit Serveridentität.
    let web = e.doc(SeedUser::Bob, "WEB", "W", "x").await;
    let mut carol = e.join("carol", web).await;
    let mut bob2 = e.join("bob", web).await;
    carol
        .send(&aw(7, 1, json!({"user": {"name": "X"}}).to_string()))
        .await;
    bob2.pump(Duration::from_millis(400)).await;
    let (_, j) = bob2.awareness.iter().find(|(id, _)| *id == 7).unwrap();
    assert_ne!(
        serde_json::from_str::<Value>(j).unwrap()["user"]["name"],
        "X"
    );

    // Zu große Awareness.
    let mut c = e.join("alice", doc).await;
    c.send(&aw(9, 1, format!("{{\"a\":\"{}\"}}", "y".repeat(10_000))))
        .await;
    c.pump(Duration::from_millis(400)).await;
    assert!(c.closed.is_some());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn message_rate_is_limited(pool: PgPool) {
    let e = env_with(
        pool,
        CollabConfig {
            messages_per_sec: 1,
            message_burst: 5,
            ..collab_cfg()
        },
    )
    .await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    let mut c = e.join("alice", doc).await;
    for _ in 0..9 {
        c.send(&YMessage::Sync(SyncMessage::SyncStep1(
            StateVector::default(),
        )))
        .await;
    }
    c.pump(Duration::from_millis(500)).await;
    assert!(matches!(c.closed, Some(4429) | Some(0)), "{:?}", c.closed);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn connection_limits_per_room_and_user(pool: PgPool) {
    let e = env_with(
        pool,
        CollabConfig {
            max_connections_per_room: 2,
            max_connections_per_user: 3,
            ..collab_cfg()
        },
    )
    .await;
    let doc = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    let _a = e.join("alice", doc).await;
    let _b = e.join("bob", doc).await;
    // Dritte Verbindung in den Raum: abgewiesen.
    let ws = e
        .try_connect(Some("alice"), &format!("/collab/document/{doc}"), &[])
        .await
        .unwrap();
    let mut c = Client {
        ws,
        doc: crdt::new_doc(),
        hello: Value::Null,
        awareness: vec![],
        closed: None,
    };
    c.pump(Duration::from_millis(500)).await;
    assert_eq!(c.closed, Some(1013));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn user_connection_limit_is_enforced(pool: PgPool) {
    let e = env_with(
        pool,
        CollabConfig {
            max_connections_per_user: 2,
            ..collab_cfg()
        },
    )
    .await;
    let d1 = e.doc(SeedUser::Alice, "ENG", "Eins", "1").await;
    let d2 = e.doc(SeedUser::Alice, "ENG", "Zwei", "2").await;
    let d3 = e.doc(SeedUser::Alice, "ENG", "Drei", "3").await;
    let _c1 = e.join("alice", d1).await;
    let _c2 = e.join("alice", d2).await;
    assert_eq!(
        e.status(Some("alice"), &format!("/collab/document/{d3}"))
            .await,
        429
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn revoked_token_and_lost_membership_close_the_connection(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.doc(SeedUser::Bob, "DES", "Design", "privat").await;
    let mut bob = e.join("bob", doc).await;
    assert_eq!(bob.hello["canWrite"], true);
    // Mitgliedschaft entfällt: Sicht/Schreibrecht weg.
    sqlx::query("DELETE FROM team_membership WHERE team_id = $1 AND user_id = $2")
        .bind(e.d.des.0)
        .bind(e.d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    bob.pump(Duration::from_secs(2)).await;
    assert!(
        matches!(bob.closed, Some(4403) | Some(4404)),
        "{:?}",
        bob.closed
    );

    // Token widerrufen -> 4401.
    let doc2 = e.doc(SeedUser::Alice, "ENG", "Spec", "x").await;
    let mut alice = e.join("alice", doc2).await;
    sqlx::query("DELETE FROM user_session WHERE token_hash = $1")
        .bind(lineal_app::usecases::auth::hash_token(&token("alice")))
        .execute(&e.pool)
        .await
        .unwrap();
    alice.pump(Duration::from_secs(2)).await;
    assert_eq!(alice.closed, Some(4401));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comment_anchors_survive_edits_and_detach_when_text_is_removed(pool: PgPool) {
    use lineal_app::ports::repos::user::OrderBy;
    use lineal_app::usecases::comment::{CreateCommentInput, ListCommentsInput, ParentSpec};
    let e = env(pool).await;
    let doc = e
        .doc(SeedUser::Alice, "ENG", "Spec", "Hallo schöne Welt")
        .await;
    let c = e
        .app
        .create_comment(
            &user(SeedUser::Bob),
            CreateCommentInput {
                parent: Some(ParentSpec::Document(doc.to_string())),
                reply_to: None,
                body: "Warum schön?".into(),
                quoted_text: Some("schöne".into()),
            },
        )
        .await
        .unwrap();
    let id = c.view.comment.id;
    let list = |e: &Env| {
        let app = e.app.clone();
        async move {
            app.list_comments(
                &user(SeedUser::Alice),
                ListCommentsInput {
                    parent: ParentSpec::Document(doc.to_string()),
                    order_by: OrderBy::CreatedAt,
                    cursor: None,
                    limit: 50,
                },
            )
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|v| v.comment.id == id)
            .unwrap()
        }
    };
    // Altbestand ohne Anker: Fallback über den Klartext.
    let v = list(&e).await;
    assert_eq!(v.comment.anchor_start, None);
    assert_eq!(v.anchor_state, Some("attached"));

    // Beim Öffnen des Raums werden die Anker aus quotedText abgeleitet.
    let first = e.join("alice", doc).await;
    drop(first);
    let v = list(&e).await;
    assert!(v.comment.anchor_start.is_some() && v.comment.anchor_end.is_some());
    assert_eq!(v.anchor_state, Some("attached"));

    // Einfügungen davor (API): der Anker wandert mit; quotedText bleibt unverändert.
    let save = |content: &str| SaveDocumentInput {
        id: Some(doc.to_string()),
        content: Some(content.into()),
        ..Default::default()
    };
    e.app
        .save_document(&user(SeedUser::Bob), save("Vorwort Hallo schöne Welt"))
        .await
        .unwrap();
    let v = list(&e).await;
    assert_eq!(v.anchor_state, Some("attached"));
    assert_eq!(v.comment.quoted_text.as_deref(), Some("schöne"));

    // Der markierte Text verschwindet: detached, der Kommentar bleibt erhalten.
    e.app
        .save_document(&user(SeedUser::Bob), save("Vorwort Hallo Welt"))
        .await
        .unwrap();
    let v = list(&e).await;
    assert_eq!(v.anchor_state, Some("detached"));

    // Ungültige Anker werden abgelehnt.
    let err = e
        .app
        .set_comment_anchor(&user(SeedUser::Bob), &id.to_string(), "!!!", "???")
        .await
        .unwrap_err();
    assert_eq!(err.kind, lineal_app::ErrorKind::InvalidRequest);
}

async fn desc_client(e: &Env, who: &str, kind: &str, id: Uuid) -> Client {
    let ws = e
        .try_connect(Some(who), &format!("/collab/{kind}/{id}"), &[])
        .await
        .unwrap();
    Client::start(ws).await
}

async fn wait_sql<F: Fn(&str) -> bool>(e: &Env, sql: &str, id: Uuid, f: F) -> String {
    for _ in 0..80 {
        let v: Option<String> = sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(&e.pool)
            .await
            .unwrap();
        let v = v.unwrap_or_default();
        if f(&v) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("not persisted");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_description_room_converges_persists_and_merges_api_writes(pool: PgPool) {
    use lineal_app::usecases::issue::{CreateIssueInput, DescriptionEdit, UpdateIssueInput};
    let e = env(pool).await;
    let issue = e
        .app
        .create_issue(
            &user(SeedUser::Alice),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Mit Beschreibung".into()),
                description: Some("Anfang".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .issue;
    let id = issue.id.0;
    let v0 = issue.version;
    let mut alice = desc_client(&e, "alice", "issue", id).await;
    let mut bob = desc_client(&e, "bob", "issue", id).await;
    assert_eq!(alice.hello["canWrite"], true);
    assert_eq!(alice.md(), "Anfang");
    alice.add_paragraph("von Alice").await;
    bob.add_paragraph("von Bob").await;
    pump_both(&mut alice, &mut bob, 600).await;
    assert_eq!(alice.md(), bob.md());
    let text = wait_sql(&e, "SELECT description FROM issue WHERE id = $1", id, |d| {
        d.contains("von Alice") && d.contains("von Bob")
    })
    .await;
    assert!(text.ends_with("Anfang"));
    let v1: i32 = sqlx::query_scalar("SELECT version FROM issue WHERE id = $1")
        .bind(id)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert!(
        v1 > v0,
        "Issue-Version wurde über den normalen Änderungsweg erhöht"
    );

    // API-Schreibvorgang (Beschreibung ersetzen) bei offener Sitzung landet im Raum.
    e.app
        .update_issue(
            &user(SeedUser::Bob),
            UpdateIssueInput {
                id: id.to_string(),
                description: Some(DescriptionEdit::Set("Neu per API".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for _ in 0..40 {
        alice.pump(Duration::from_millis(100)).await;
        if alice.md() == "Neu per API" {
            break;
        }
    }
    assert_eq!(alice.md(), "Neu per API");

    // Gast Carol (nur WEB) sieht ENG-Issues nicht: 404.
    assert_eq!(
        e.status(Some("carol"), &format!("/collab/issue/{id}"))
            .await,
        404
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_and_initiative_description_rooms_persist_to_their_owner(pool: PgPool) {
    use lineal_testkit::seed::planning::{seed_initiative_id, seed_project_id};
    let e = env(pool).await;
    let project = seed_project_id().0;
    let mut a = desc_client(&e, "alice", "project", project).await;
    assert_eq!(a.hello["canWrite"], true);
    a.add_paragraph("Projekt-Notiz").await;
    a.pump(Duration::from_millis(300)).await;
    let text = wait_sql(
        &e,
        "SELECT content FROM project WHERE id = $1",
        project,
        |d| d.contains("Projekt-Notiz"),
    )
    .await;
    assert!(text.contains("Projekt-Notiz"));

    let initiative = seed_initiative_id().0;
    let mut b = desc_client(&e, "alice", "initiative", initiative).await;
    b.add_paragraph("Initiativ-Notiz").await;
    b.pump(Duration::from_millis(300)).await;
    wait_sql(
        &e,
        "SELECT content FROM initiative WHERE id = $1",
        initiative,
        |d| d.contains("Initiativ-Notiz"),
    )
    .await;
    // Gast ohne Sicht auf Initiativen: 404.
    assert_eq!(
        e.status(Some("carol"), &format!("/collab/initiative/{initiative}"))
            .await,
        404
    );
}

async fn issue_for(e: &Env, who: SeedUser, team: &str, title: &str) -> Uuid {
    use lineal_app::usecases::issue::CreateIssueInput;
    e.app
        .create_issue(
            &user(who),
            CreateIssueInput {
                team: team.into(),
                title: Some(title.into()),
                description: Some("Beschreibung".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .issue
        .id
        .0
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn shared_issue_is_read_only_and_access_ends_when_sharing_is_switched_off(pool: PgPool) {
    let e = env(pool).await;
    sqlx::query(
        r#"UPDATE organization SET limits = limits || '{"issueSharingEnabled": true}'::jsonb"#,
    )
    .execute(&e.pool)
    .await
    .unwrap();
    let id = issue_for(&e, SeedUser::Alice, "ENG", "Geteilt").await;
    // Ohne Freigabe: Carol (Gast, nur WEB) sieht das ENG-Issue nicht.
    assert_eq!(
        e.status(Some("carol"), &format!("/collab/issue/{id}"))
            .await,
        404
    );
    e.app
        .share_issue(
            &user(SeedUser::Alice),
            &id.to_string(),
            "carol@acme.example",
        )
        .await
        .unwrap();
    // Mit Freigabe: lesen ja, schreiben nein (Update wird verworfen, Verbindung geschlossen).
    let mut carol = desc_client(&e, "carol", "issue", id).await;
    assert_eq!(carol.hello["canWrite"], false);
    assert_eq!(carol.md(), "Beschreibung");
    carol.add_paragraph("Vandalismus").await;
    carol.pump(Duration::from_millis(500)).await;
    assert_eq!(carol.closed, Some(4403));
    tokio::time::sleep(Duration::from_millis(500)).await;
    let d: Option<String> = sqlx::query_scalar("SELECT description FROM issue WHERE id = $1")
        .bind(id)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(d.as_deref(), Some("Beschreibung"));

    // Freigaben werden während der offenen Sitzung ausgeschaltet: kein Zugriff mehr, Trennung.
    let mut carol = desc_client(&e, "carol", "issue", id).await;
    sqlx::query(
        r#"UPDATE organization SET limits = limits || '{"issueSharingEnabled": false}'::jsonb"#,
    )
    .execute(&e.pool)
    .await
    .unwrap();
    carol.pump(Duration::from_secs(3)).await;
    assert!(
        matches!(carol.closed, Some(4404) | Some(4403)),
        "{:?}",
        carol.closed
    );
    assert_eq!(
        e.status(Some("carol"), &format!("/collab/issue/{id}"))
            .await,
        404
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_scoped_key_never_gets_access_outside_its_teams_not_even_via_share(pool: PgPool) {
    let e = env(pool).await;
    sqlx::query(
        r#"UPDATE organization SET limits = limits || '{"issueSharingEnabled": true}'::jsonb"#,
    )
    .execute(&e.pool)
    .await
    .unwrap();
    // Bobs Key ist auf WEB beschränkt; Bob ist auch ENG-Mitglied.
    sqlx::query(
        "INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash, scopes, team_ids) \
         VALUES ($1, $2, 'web only', 'lin_wo', $3, ARRAY['read','write'], ARRAY[$4::uuid])",
    )
    .bind(e.d.org.0)
    .bind(e.d.bob.0)
    .bind(lineal_app::usecases::auth::hash_token("lin_web_only_key"))
    .bind(e.d.web.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let eng_doc = e.doc(SeedUser::Alice, "ENG", "Intern", "x").await;
    let eng_issue = issue_for(&e, SeedUser::Alice, "ENG", "Intern").await;
    // Auch eine Freigabe an Bob hilft dem team-beschränkten Key nicht.
    e.app
        .share_issue(
            &user(SeedUser::Alice),
            &eng_issue.to_string(),
            "bob@acme.example",
        )
        .await
        .unwrap();
    for path in [
        format!("/collab/document/{eng_doc}"),
        format!("/collab/issue/{eng_issue}"),
    ] {
        assert_eq!(
            e.status(Some("key:lin_web_only_key"), &path).await,
            404,
            "{path}"
        );
        // Dieselben Objekte sind mit Bobs Session erreichbar.
        assert_eq!(e.status(Some("bob"), &path).await, 101, "{path}");
    }
    // Innerhalb des Scopes funktioniert der Key.
    let web_doc = e.doc(SeedUser::Bob, "WEB", "Web", "x").await;
    assert_eq!(
        e.status(
            Some("key:lin_web_only_key"),
            &format!("/collab/document/{web_doc}")
        )
        .await,
        101
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn suspended_user_trashed_and_archived_owner_objects_end_or_downgrade_access(pool: PgPool) {
    let e = env(pool).await;
    // Gesperrter User: Token gilt nicht mehr -> 4401.
    let id = issue_for(&e, SeedUser::Alice, "ENG", "Sperre").await;
    let mut bob = desc_client(&e, "bob", "issue", id).await;
    sqlx::query(r#"UPDATE "user" SET active = false WHERE id = $1"#)
        .bind(e.d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    bob.pump(Duration::from_secs(3)).await;
    assert_eq!(bob.closed, Some(4401));
    sqlx::query(r#"UPDATE "user" SET active = true WHERE id = $1"#)
        .bind(e.d.bob.0)
        .execute(&e.pool)
        .await
        .unwrap();

    // Archiviertes Issue: schreibgeschützt (Schreiber werden mit 4403 getrennt, Neubeitritt read-only).
    let archived = issue_for(&e, SeedUser::Alice, "ENG", "Archiv").await;
    let mut alice = desc_client(&e, "alice", "issue", archived).await;
    assert_eq!(alice.hello["canWrite"], true);
    e.app
        .archive_issue(&user(SeedUser::Alice), &archived.to_string())
        .await
        .unwrap();
    alice.pump(Duration::from_secs(3)).await;
    assert_eq!(alice.closed, Some(4403));
    let again = desc_client(&e, "alice", "issue", archived).await;
    assert_eq!(again.hello["canWrite"], false);

    // Gelöschtes (Papierkorb) Issue: nicht mehr vorhanden -> 4404, kein Beitritt.
    let trashed = issue_for(&e, SeedUser::Alice, "ENG", "Papierkorb").await;
    let mut alice = desc_client(&e, "alice", "issue", trashed).await;
    e.app
        .delete_issue(&user(SeedUser::Alice), &trashed.to_string(), false)
        .await
        .unwrap();
    alice.pump(Duration::from_secs(3)).await;
    assert_eq!(alice.closed, Some(4404));
    assert_eq!(
        e.status(Some("alice"), &format!("/collab/issue/{trashed}"))
            .await,
        404
    );
}
