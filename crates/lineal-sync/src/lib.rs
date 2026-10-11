//! Sync-Adapter (WP15, docs/09 §5, docs/11 §3.7): `GET /sync?since=<lastSyncId>` öffnet einen
//! WebSocket. Der Server liefert zuerst die fehlenden `SyncAction`s aus dem `sync_log`, danach live
//! alle neuen. Geweckt wird per PostgreSQL `LISTEN/NOTIFY` (Kanal `lineal_sync`), zusätzlich pollt
//! jede Verbindung langsam als Rückfall.
//!
//! **Sicherheit.** Die Authentifizierung läuft vor dem Upgrade (Session-Cookie oder API-Key/OAuth-Token
//! wie in der API; ohne gültige Zugangsdaten `401`). Jede Aktion trägt Sync-Gruppen
//! (`team:<id>`, `org`, `user:<id>`); zugestellt wird nur, was sich mit den Gruppen des Users
//! **zum Zustellzeitpunkt** überschneidet (Team-Sichtbarkeit wird bei jedem Zustellvorgang neu
//! berechnet, also verliert ein aus einem privaten Team entfernter User sofort den Strom dieses
//! Teams). Persönliche Objekte (Notifications) tragen nur `user:<id>`.
//!
//! **Re-Validierung.** Das Token der Verbindung wird alle `revalidate_interval` (60 s) und nach
//! jeder Zustellung von User-/Team-/Organisations-Änderungen neu geprüft; widerrufene oder
//! abgelaufene Tokens und suspendierte User beenden die Verbindung mit Close-Code 4401.
//!
//! **Protokoll** (JSON-Textframes):
//! - Server → Client: `{"type":"hello","lastSyncId":N,"resumed":bool}`,
//!   `{"type":"sync","actions":[…],"lastSyncId":N}`, `{"type":"pong","lastSyncId":N}`,
//!   `{"type":"resetRequired"}` (danach schließt der Server mit Code 4001).
//! - Client → Server: `{"type":"ping"}`.
//! - Aktion: `{"id","action":"I|U|A|V|D","modelName","modelId","data","syncGroups","syncId","transactionId":null}`;
//!   `id` ist der Stream-Cursor (streng monoton, Lücken durch den Gruppenfilter sind normal).
//!
//! **Grenzen.** Verbindungen je User, Größe eingehender Nachrichten, Größe ausgehender Pakete,
//! Ping/Idle-Timeout und ein Sende-Timeout (langsame Clients werden getrennt).
#![forbid(unsafe_code)]

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::header::{AUTHORIZATION, COOKIE, HOST, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use lineal_app::policy::PolicySnapshot;
use lineal_app::ports::repos::outbox::SyncActionRow;
use lineal_app::usecases::auth::hash_token;
use lineal_app::{Actor, App, AppError, ErrorKind, RequestCtx};
use lineal_domain::enums::UserRole;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::{PgListener, PgPoolOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{Instant, interval_at, timeout};
use uuid::Uuid;

/// Name des Session-Cookies (identisch zur API).
const SESSION_COOKIE: &str = "lineal_session";
/// Längere Tokens sind nie gültig.
const MAX_TOKEN_LEN: usize = 512;
/// `NOTIFY`-Kanal, auf den `sync_log`-Schreiber senden.
pub const NOTIFY_CHANNEL: &str = "lineal_sync";
/// WebSocket-Schließcode für „Resync nötig“.
pub const CLOSE_RESET_REQUIRED: u16 = 4001;
/// WebSocket-Schließcode „Policy Violation“ (Auth/Sichtbarkeit nicht mehr gegeben).
const CLOSE_POLICY: u16 = 1008;
/// WebSocket-Schließcode, wenn das Token der Verbindung nicht mehr gilt (widerrufen, abgelaufen,
/// User suspendiert). Der Client muss sich neu anmelden, nicht nur neu verbinden.
pub const CLOSE_UNAUTHORIZED: u16 = 4401;
/// Sync-Modelle, deren Änderung Mitgliedschaft/Rolle/Sichtbarkeit berühren können: nach einer
/// Zustellung solcher Aktionen wird das Token sofort neu geprüft.
const MEMBERSHIP_MODELS: [&str; 3] = ["User", "Team", "Organization"];

/// Einstellungen des Sync-Adapters.
#[derive(Clone)]
pub struct SyncConfig {
    /// Gleichzeitige Verbindungen je User; weitere werden mit 429 abgewiesen.
    pub max_connections_per_user: usize,
    /// Maximale Größe einer Client-Nachricht in Bytes (Frames darüber beenden die Verbindung).
    pub max_incoming_bytes: usize,
    /// Zielgröße eines `sync`-Pakets in Bytes (eine einzelne größere Aktion geht allein).
    pub max_outgoing_bytes: usize,
    /// Aktionen je Datenbank-Abruf.
    pub batch_actions: i64,
    /// Abstand der Server-Pings.
    pub ping_interval: Duration,
    /// Ohne Lebenszeichen (jeder Frame) so lange → Verbindung wird beendet.
    pub idle_timeout: Duration,
    /// Dauert das Senden eines Frames länger (langsamer Client), wird getrennt.
    pub send_timeout: Duration,
    /// Rückfall-Polling, falls `NOTIFY` nicht ankommt.
    pub poll_interval: Duration,
    /// Abstand der periodischen Token-Prüfung laufender Verbindungen.
    pub revalidate_interval: Duration,
    /// Pool für `LISTEN`; `None` → nur Polling.
    pub listen_pool: Option<PgPool>,
    /// Zusätzlich erlaubte `Origin`-Werte (z. B. Dev-Server); gleicher Host/`base_url` ist immer erlaubt.
    pub extra_origins: Vec<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_connections_per_user: 10,
            max_incoming_bytes: 16 * 1024,
            max_outgoing_bytes: 512 * 1024,
            batch_actions: 200,
            ping_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(90),
            send_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_secs(2),
            revalidate_interval: Duration::from_secs(60),
            listen_pool: None,
            extra_origins: vec![],
        }
    }
}

impl SyncConfig {
    /// Standardwerte; `LISTEN` über `DATABASE_URL`, falls gesetzt.
    pub fn from_env() -> Self {
        let listen_pool = std::env::var("DATABASE_URL").ok().and_then(|url| {
            PgPoolOptions::new()
                .max_connections(1)
                .connect_lazy(&url)
                .ok()
        });
        Self {
            listen_pool,
            ..Self::default()
        }
    }
}

struct Shared {
    app: Arc<App>,
    cfg: SyncConfig,
    wake: broadcast::Sender<()>,
    users: Mutex<HashMap<Uuid, usize>>,
    listener: OnceLock<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    /// Reserviert einen Verbindungsplatz für den User.
    fn acquire(self: &Arc<Self>, user: Uuid) -> Option<ConnGuard> {
        let mut users = lock(&self.users);
        let n = users.entry(user).or_insert(0);
        if *n >= self.cfg.max_connections_per_user {
            return None;
        }
        *n += 1;
        Some(ConnGuard {
            shared: self.clone(),
            user,
        })
    }

    /// Startet den `LISTEN`-Task beim ersten Verbindungsaufbau.
    fn ensure_listener(&self) {
        if let Some(pool) = &self.cfg.listen_pool {
            self.listener.get_or_init(|| {
                tokio::spawn(listen_loop(pool.clone(), self.wake.clone()));
            });
        }
    }
}

struct ConnGuard {
    shared: Arc<Shared>,
    user: Uuid,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut users = lock(&self.shared.users);
        if let Some(n) = users.get_mut(&self.user) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                users.remove(&self.user);
            }
        }
    }
}

/// Hält eine `LISTEN`-Verbindung und weckt alle WebSocket-Verbindungen bei `NOTIFY`.
async fn listen_loop(pool: PgPool, wake: broadcast::Sender<()>) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match PgListener::connect_with(&pool).await {
            Ok(mut listener) => match listener.listen(NOTIFY_CHANNEL).await {
                Ok(()) => {
                    backoff = Duration::from_millis(500);
                    // Nach (Wieder-)Verbinden alles nachholen, was währenddessen geschrieben wurde.
                    let _ = wake.send(());
                    loop {
                        match listener.recv().await {
                            Ok(_) => {
                                let _ = wake.send(());
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "sync LISTEN connection lost");
                                break;
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "sync LISTEN failed"),
            },
            Err(e) => tracing::warn!(error = %e, "sync listener cannot connect (polling only)"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// Routen des Sync-Adapters (Konfiguration aus der Umgebung).
pub fn router(app: Arc<App>) -> Router {
    router_with(app, SyncConfig::from_env())
}

/// Wie [`router`] mit eigener Konfiguration (Tests).
pub fn router_with(app: Arc<App>, cfg: SyncConfig) -> Router {
    let (wake, _) = broadcast::channel(16);
    let shared = Arc::new(Shared {
        app,
        cfg,
        wake,
        users: Mutex::new(HashMap::new()),
        listener: OnceLock::new(),
    });
    Router::new()
        .route("/sync", get(sync_socket))
        .with_state(shared)
}

// --- Authentifizierung -------------------------------------------------------------------

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "error": code, "message": message, "status": status.as_u16() })),
    )
        .into_response()
}

fn app_error_response(e: &AppError) -> Response {
    if e.kind == ErrorKind::Internal {
        tracing::error!(error = %e.message, "sync auth lookup failed");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Internal error.",
        );
    }
    let status = StatusCode::from_u16(e.kind.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    error_response(status, e.kind.code(), &e.message)
}

/// Token aus `Authorization` (Bearer oder roh) oder dem Session-Cookie.
fn extract_token(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        let raw = match value.split_once(' ') {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => rest.trim(),
            _ => value,
        };
        if !raw.is_empty() {
            return Some(raw.to_owned());
        }
    }
    for header in headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for pair in header.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=')
                && name == SESSION_COOKIE
                && !value.is_empty()
            {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Löst die Zugangsdaten auf; liefert `RequestCtx` und den Token-Hash (für die Re-Validierung).
async fn authenticate(app: &App, headers: &HeaderMap) -> Result<(RequestCtx, String), Response> {
    let unauthenticated = |msg: &str| {
        let mut res = error_response(StatusCode::UNAUTHORIZED, "unauthenticated", msg);
        res.headers_mut().insert(
            WWW_AUTHENTICATE,
            "Bearer".parse().expect("static header value"),
        );
        res
    };
    let Some(token) = extract_token(headers) else {
        return Err(unauthenticated("Authentication required."));
    };
    if token.len() > MAX_TOKEN_LEN {
        return Err(unauthenticated("Invalid or expired token."));
    }
    let token_hash = hash_token(&token);
    let principal = lookup(app, &token_hash)
        .await
        .map_err(|e| app_error_response(&e))?
        .ok_or_else(|| unauthenticated("Invalid or expired token."))?;
    let ctx = RequestCtx {
        org_id: principal.org_id,
        actor: principal.actor,
        request_id: format!("req_sync_{}", Uuid::now_v7().simple()),
        scopes: principal.scopes,
        team_scope: principal.team_scope.clone(),

        via_session: false,
    };
    Ok((ctx, token_hash))
}

async fn lookup(
    app: &App,
    token_hash: &str,
) -> Result<Option<lineal_app::ports::repos::Principal>, AppError> {
    let mut tx = app.store.begin_system().await?;
    tx.auth().find_principal_by_token_hash(token_hash).await
}

/// Prüft das Token einer laufenden Verbindung erneut (Widerruf, Ablauf, Suspendierung,
/// Rollenwechsel) und übernimmt Rolle/Scopes. `Unauthorized`, wenn es nicht mehr gilt.
async fn revalidate(shared: &Shared, token_hash: &str, ctx: &mut RequestCtx) -> Result<(), End> {
    match lookup(&shared.app, token_hash).await? {
        Some(p) if p.org_id == ctx.org_id && p.scopes.read => {
            ctx.actor = p.actor;
            ctx.scopes = p.scopes;
            Ok(())
        }
        _ => Err(End::Unauthorized),
    }
}

/// Schutz vor Cross-Site-WebSocket-Hijacking: ein gesendeter `Origin` muss zum Host, zur
/// `base_url` oder zu den konfigurierten Zusatz-Ursprüngen passen.
fn origin_allowed(headers: &HeaderMap, shared: &Shared) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // Nicht-Browser-Clients senden keinen Origin
    };
    let authority = |url: &str| {
        url.split_once("://")
            .map_or(url, |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let origin_auth = authority(origin);
    if origin_auth.is_empty() {
        return false;
    }
    if shared
        .cfg
        .extra_origins
        .iter()
        .any(|o| o.eq_ignore_ascii_case(origin))
    {
        return true;
    }
    if origin_auth == authority(&shared.app.config.base_url) {
        return true;
    }
    headers
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| h.eq_ignore_ascii_case(&origin_auth))
}

#[derive(Deserialize)]
struct SyncQuery {
    since: Option<i64>,
}

async fn sync_socket(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    // Reihenfolge: erst Auth (401), dann Upgrade-Fehler.
    let (ctx, token_hash) = match authenticate(&shared.app, &headers).await {
        Ok(pair) => pair,
        Err(res) => return res,
    };
    let ws = match ws {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    if !origin_allowed(&headers, &shared) {
        return error_response(StatusCode::FORBIDDEN, "forbidden", "Origin not allowed.");
    }
    if !ctx.scopes.read {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Token has no read scope.",
        );
    }
    let user = match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => id.0,
        Actor::System => {
            return error_response(StatusCode::FORBIDDEN, "forbidden", "No user context.");
        }
    };
    if query.since.is_some_and(|s| s < 0) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "`since` must not be negative.",
        );
    }
    let Some(guard) = shared.acquire(user) else {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many sync connections for this user.",
        );
    };
    shared.ensure_listener();
    let max = shared.cfg.max_incoming_bytes;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| connection(socket, shared, (ctx, token_hash), query.since, guard))
}

// --- Verbindung --------------------------------------------------------------------------

/// Warum eine Verbindung endet.
#[derive(Debug)]
enum End {
    /// Client weg oder normal geschlossen.
    Closed,
    /// Client zu langsam.
    Slow,
    /// Keine Lebenszeichen.
    Idle,
    /// Token widerrufen/abgelaufen oder User suspendiert (Close 4401).
    Unauthorized,
    /// Fehler beim Zustellen.
    Error(AppError),
}

impl From<AppError> for End {
    fn from(e: AppError) -> Self {
        Self::Error(e)
    }
}

async fn connection(
    mut socket: WebSocket,
    shared: Arc<Shared>,
    (mut ctx, token_hash): (RequestCtx, String),
    since: Option<i64>,
    _guard: ConnGuard,
) {
    match drive(&mut socket, &shared, &mut ctx, &token_hash, since).await {
        Ok(()) | Err(End::Closed) => {}
        Err(End::Unauthorized) => {
            tracing::info!("sync token no longer valid, disconnecting");
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: CLOSE_UNAUTHORIZED,
                    reason: "unauthorized".into(),
                })))
                .await;
        }
        Err(End::Slow) => tracing::info!("sync client too slow, disconnecting"),
        Err(End::Idle) => {
            tracing::debug!("sync client idle, disconnecting");
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: 1001,
                    reason: "idle timeout".into(),
                })))
                .await;
        }
        Err(End::Error(e)) => {
            tracing::warn!(error = %e, "sync connection ended with error");
            let code = match e.kind {
                ErrorKind::Internal => 1011,
                _ => CLOSE_POLICY,
            };
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code,
                    reason: "sync ended".into(),
                })))
                .await;
        }
    }
}

async fn send_text(socket: &mut WebSocket, cfg: &SyncConfig, value: &Value) -> Result<(), End> {
    send(socket, cfg, Message::Text(value.to_string().into())).await
}

async fn send(socket: &mut WebSocket, cfg: &SyncConfig, msg: Message) -> Result<(), End> {
    match timeout(cfg.send_timeout, socket.send(msg)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(End::Closed),
        Err(_) => Err(End::Slow),
    }
}

async fn drive(
    socket: &mut WebSocket,
    shared: &Shared,
    ctx: &mut RequestCtx,
    token_hash: &str,
    since: Option<i64>,
) -> Result<(), End> {
    let cfg = &shared.cfg;
    // Zuerst abonnieren, dann Kopf lesen: nichts geht zwischen Backlog und Live verloren.
    let mut wake = shared.wake.subscribe();
    let head = {
        let mut tx = shared.app.store.begin(ctx.org_id).await?;
        tx.outbox().sync_head().await?
    };
    let mut cursor = match since {
        Some(s) if s < head.purged_up_to || s > head.head => {
            send_text(socket, cfg, &json!({ "type": "resetRequired" })).await?;
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: CLOSE_RESET_REQUIRED,
                    reason: "resync required".into(),
                })))
                .await;
            return Ok(());
        }
        Some(s) => s,
        None => head.head,
    };
    send_text(
        socket,
        cfg,
        &json!({ "type": "hello", "lastSyncId": head.head, "resumed": since.is_some() }),
    )
    .await?;
    deliver(socket, shared, ctx, token_hash, &mut cursor).await?;

    let start = Instant::now();
    let mut ping = interval_at(start + cfg.ping_interval, cfg.ping_interval);
    let mut poll = interval_at(start + cfg.poll_interval, cfg.poll_interval);
    let mut recheck = interval_at(start + cfg.revalidate_interval, cfg.revalidate_interval);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return Ok(()) };
                last_seen = Instant::now();
                match msg {
                    Message::Text(text) => {
                        if serde_json::from_str::<Value>(&text)
                            .is_ok_and(|v| v["type"] == "ping")
                        {
                            send_text(socket, cfg, &json!({ "type": "pong", "lastSyncId": cursor })).await?;
                        }
                    }
                    Message::Close(_) => return Ok(()),
                    Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            r = wake.recv() => {
                match r {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        deliver(socket, shared, ctx, token_hash, &mut cursor).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            _ = poll.tick() => deliver(socket, shared, ctx, token_hash, &mut cursor).await?,
            _ = recheck.tick() => revalidate(shared, token_hash, ctx).await?,
            _ = ping.tick() => {
                if last_seen.elapsed() > cfg.idle_timeout {
                    return Err(End::Idle);
                }
                send(socket, cfg, Message::Ping(Vec::new().into())).await?;
            }
        }
    }
}

/// Erlaubte Gruppen eines Users **jetzt**: sichtbare Teams, Workspace (nicht für Gäste), persönlich.
fn allowed_groups(snap: &PolicySnapshot, ctx: &RequestCtx) -> Vec<String> {
    let mut groups: Vec<String> = snap
        .visible_team_ids()
        .into_iter()
        .map(|t| format!("team:{}", t.0))
        .collect();
    if let Actor::User { id, role } = ctx.actor {
        if role != UserRole::Guest {
            groups.push("org".to_owned());
        }
        groups.push(format!("user:{}", id.0));
    } else if let Actor::App { id } = ctx.actor {
        groups.push("org".to_owned());
        groups.push(format!("user:{}", id.0));
    }
    groups
}

struct Batch {
    head: i64,
    rows: Vec<SyncActionRow>,
    groups: Vec<String>,
}

/// Liest den nächsten Block sichtbarer Aktionen (eigene kurze Transaktion, Sichtbarkeit frisch).
async fn fetch(shared: &Shared, ctx: &RequestCtx, cursor: i64) -> Result<Option<Batch>, AppError> {
    let mut tx = shared.app.store.begin(ctx.org_id).await?;
    let head = tx.outbox().sync_head().await?.head;
    if head <= cursor {
        return Ok(None);
    }
    let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
    let groups = allowed_groups(&snap, ctx);
    let rows = tx
        .outbox()
        .sync_read(cursor, head, &groups, shared.cfg.batch_actions)
        .await?;
    Ok(Some(Batch { head, rows, groups }))
}

fn wire(row: &SyncActionRow, allowed: &[String]) -> Value {
    // Gruppen auf die des Empfängers beschränken: fremde Team-IDs werden nicht verraten.
    let groups: Vec<&String> = row.groups.iter().filter(|g| allowed.contains(g)).collect();
    json!({
        "id": row.id,
        "syncId": row.sync_id,
        "action": row.action,
        "modelName": row.model_name,
        "modelId": row.model_id,
        "data": row.data,
        "syncGroups": groups,
        "transactionId": Value::Null,
    })
}

/// Stellt alle neuen sichtbaren Aktionen zu und rückt den Cursor vor.
async fn deliver(
    socket: &mut WebSocket,
    shared: &Shared,
    ctx: &mut RequestCtx,
    token_hash: &str,
    cursor: &mut i64,
) -> Result<(), End> {
    let cfg = &shared.cfg;
    loop {
        let Some(batch) = fetch(shared, ctx, *cursor).await? else {
            return Ok(());
        };
        // Änderungen an User/Team/Organisation können Mitgliedschaft oder Rolle betreffen:
        // Token sofort neu prüfen, bevor weitere Aktionen zugestellt werden.
        if batch
            .rows
            .iter()
            .any(|r| MEMBERSHIP_MODELS.contains(&r.model_name.as_str()))
        {
            revalidate(shared, token_hash, ctx).await?;
        }
        let full = i64::try_from(batch.rows.len()).unwrap_or(i64::MAX) >= cfg.batch_actions;
        let new_cursor = if full {
            batch.rows.last().map_or(batch.head, |r| r.id)
        } else {
            batch.head
        };
        // In Pakete bis ca. `max_outgoing_bytes` zerlegen.
        let mut chunk: Vec<Value> = Vec::new();
        let mut size = 0usize;
        let mut last_id = *cursor;
        let flush = |chunk: &mut Vec<Value>, last_id: i64| json!({ "type": "sync", "actions": std::mem::take(chunk), "lastSyncId": last_id });
        for row in &batch.rows {
            let action = wire(row, &batch.groups);
            let len = action.to_string().len();
            if !chunk.is_empty() && size + len > cfg.max_outgoing_bytes {
                let msg = flush(&mut chunk, last_id);
                send_text(socket, cfg, &msg).await?;
                size = 0;
            }
            size += len;
            last_id = row.id;
            chunk.push(action);
        }
        if !chunk.is_empty() {
            let msg = flush(&mut chunk, new_cursor);
            send_text(socket, cfg, &msg).await?;
        }
        *cursor = new_cursor;
        if !full {
            return Ok(());
        }
    }
}
