//! Echtzeit-Mitbearbeitung (WP36, docs/06 §1.9 Stufe 2): `GET /collab/{document|issue|project|initiative}/{id}` öffnet einen
//! WebSocket in den Raum des Inhalts.
//!
//! **Warum ein eigener Endpunkt statt eines Kanals in `/sync`?** `/sync` ist ein einziger
//! JSON-Strom mit Cursor (`lastSyncId`), Backlog und Gruppenfilter je User; ein Raum dagegen ist
//! binäres y-protocols-Framing je Inhalt mit eigener Lebensdauer, eigenem Reconnect (Zustand statt
//! Cursor) und eigenen Grenzen. Beides in einer Verbindung zu mischen würde Reconnect-Semantik,
//! Backpressure und die nginx-Limits verflechten. Auth, Origin-Prüfung und Re-Validierung sind
//! dagegen identisch und werden aus dem Sync-Adapter wiederverwendet.
//!
//! **Protokoll.** Server → Client zuerst ein Textframe
//! `{"type":"hello","epoch":"<uuid>","canWrite":bool,"version":n,"user":{...}}`, danach binäre
//! Frames im y-websocket-Format (`0` Sync mit Schritt 1/2/Update, `1` Awareness, `3`
//! Awareness-Abfrage). Der Client vergleicht `epoch` mit der Epoche seines lokalen Stands und
//! verwirft diesen bei Abweichung (der Server hat die CRDT-Linie neu aufgebaut).
//! Client → Server zusätzlich `{"type":"ping"}`.
//!
//! **Sicherheit.** Auth vor dem Upgrade (401), unsichtbarer Inhalt ist `404` (wie nicht vorhanden),
//! Origin-Prüfung, Schreibrecht wird beim Beitritt und danach periodisch sowie bei jeder
//! Sync-Änderung frisch geprüft (verlorenes Token → Close 4401, verlorenes Recht → Close 4403).
//! Updates von Nur-Lesern werden verworfen und die Verbindung geschlossen (4403). Awareness wird
//! serverseitig auf die authentifizierte Identität umgeschrieben (Name, Farbe) und je Client-ID
//! genau einer Verbindung zugeordnet. Grenzen: Updategröße, Dokumentgröße, Nachrichten je Sekunde,
//! Verbindungen je Raum/User, Strukturtiefe/Knotenzahl (Verstoß: Update zurückgerollt, Close 1009).
//! Updates werden unter `catch_unwind` angewendet; ein Defekt rollt auf den letzten Prüfpunkt zurück.
//!
//! **Persistenz.** Je Raum ein Task: Speichern nach `persist_quiet` Ruhe bzw. spätestens
//! `persist_max` (über `App::collab_persist` – dieselbe Pipeline wie automatisches Speichern:
//! Bereinigung, Version, Snapshot-Bündelung, Outbox-Ereignisse, Erwähnungen), Erkennen externer
//! Änderungen (API/MCP) alle `poll_interval` mit CRDT-Zusammenführung, Entladen nach
//! `idle_unload` ohne Teilnehmer. Einzelinstanz-Annahme: Räume leben im Serverprozess.
use super::*;
use axum::extract::Path;
use lineal_app::collab::crdt;
use lineal_app::usecases::document::{CollabAccess, CollabLoad, CollabTarget, PersistOutcome};
use lineal_domain::ids::{DocumentId, InitiativeId, IssueId, ProjectId};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex as AsyncMutex, Notify, OnceCell, mpsc};
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
use yrs::sync::{Message as YMessage, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

/// Schließcode: Recht verloren bzw. Schreibversuch ohne Bearbeitungsrecht.
pub const CLOSE_FORBIDDEN: u16 = 4403;
/// Schließcode: Inhalt existiert nicht mehr oder ist nicht mehr sichtbar.
pub const CLOSE_GONE: u16 = 4404;
/// Schließcode: Rate-Limit der Verbindung überschritten.
pub const CLOSE_RATE_LIMITED: u16 = 4429;
/// Schließcode: ungültige Daten (defekte Nachricht/Update).
pub const CLOSE_INVALID: u16 = 4400;
/// Schließcode: zu groß (Update, Dokument, Struktur).
pub const CLOSE_TOO_BIG: u16 = 1009;
/// Schließcode: Raum wird neu aufgebaut (Client verbindet sich neu).
pub const CLOSE_RESTART: u16 = 4500;

/// Einstellungen der Mitbearbeitung.
#[derive(Clone)]
pub struct CollabConfig {
    /// Größe einer einzelnen Client-Nachricht (Update) in Bytes.
    pub max_update_bytes: usize,
    /// Grenze für den kodierten Dokumentzustand in Bytes.
    pub max_doc_state_bytes: usize,
    /// Nachrichten je Sekunde und Verbindung (Dauerrate) und Burst.
    pub messages_per_sec: u32,
    pub message_burst: u32,
    /// Gleichzeitige Verbindungen je Raum.
    pub max_connections_per_room: usize,
    /// Gleichzeitige Collab-Verbindungen je User (alle Räume).
    pub max_connections_per_user: usize,
    /// Gleichzeitig geladene Räume.
    pub max_rooms: usize,
    /// Ruhe nach der letzten Änderung bis zum Speichern.
    pub persist_quiet: Duration,
    /// Spätestens nach dieser Zeit seit der ersten ungespeicherten Änderung wird gespeichert.
    pub persist_max: Duration,
    /// Abstand der Prüfung auf externe Änderungen und der Raum-Wartung.
    pub poll_interval: Duration,
    /// Raum ohne Teilnehmer und ungespeicherte Änderungen wird nach dieser Zeit entladen.
    pub idle_unload: Duration,
    /// Abstand der periodischen Rechte-/Token-Prüfung einer Verbindung.
    pub revalidate_interval: Duration,
    /// Frühestens alle so viele Sekunden wird nach einer Sync-Änderung erneut geprüft.
    pub recheck_throttle: Duration,
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    pub send_timeout: Duration,
    /// Puffer ausgehender Frames je Verbindung (voll → Verbindung wird als zu langsam getrennt).
    pub outbound_buffer: usize,
}

impl Default for CollabConfig {
    fn default() -> Self {
        Self {
            max_update_bytes: 512 * 1024,
            max_doc_state_bytes: 16 * 1024 * 1024,
            messages_per_sec: 60,
            message_burst: 200,
            max_connections_per_room: 50,
            max_connections_per_user: 20,
            max_rooms: 500,
            persist_quiet: Duration::from_secs(2),
            persist_max: Duration::from_secs(10),
            poll_interval: Duration::from_secs(1),
            idle_unload: Duration::from_secs(60),
            revalidate_interval: Duration::from_secs(10),
            recheck_throttle: Duration::from_secs(2),
            ping_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(90),
            send_timeout: Duration::from_secs(10),
            outbound_buffer: 512,
        }
    }
}

type RoomKey = (Uuid, CollabTarget);

/// Raumverwaltung (Teil von `Shared`).
#[derive(Default)]
pub(crate) struct Rooms {
    map: Mutex<HashMap<RoomKey, Arc<OnceCell<Arc<Room>>>>>,
    users: Mutex<HashMap<Uuid, usize>>,
}

/// Nachricht an die Schreibschleife einer Verbindung.
enum Out {
    Frame(Vec<u8>),
    Text(String),
    Close(u16, &'static str),
}

struct ConnEntry {
    tx: mpsc::Sender<Out>,
    user: Uuid,
    ctx: RequestCtx,
    can_write: bool,
    client_ids: HashSet<u64>,
}

struct AwarenessSlot {
    clock: u32,
    json: Arc<str>,
    conn: u64,
}

struct RoomState {
    doc: Doc,
    epoch: Uuid,
    /// Inhaltsrevision, auf der der Raum aufbaut.
    version: i32,
    /// Prüfpunkt und seitdem akzeptierte Updates (Rücksetzen bei Defekt/Verstoß).
    checkpoint: Vec<u8>,
    log: Vec<Vec<u8>>,
    log_bytes: usize,
    approx_state_bytes: usize,
    conns: HashMap<u64, ConnEntry>,
    awareness: HashMap<u64, AwarenessSlot>,
    /// Zähler der Änderungen am Dokument (für „während des Speicherns geändert“).
    generation: u64,
    saved_generation: u64,
    dirty_since: Option<Instant>,
    last_change: Instant,
    /// Kontext des zuletzt schreibenden Users (Rechteprüfung beim Speichern).
    last_writer: Option<RequestCtx>,
    empty_since: Option<Instant>,
    persisting: bool,
}

pub(crate) struct Room {
    key: RoomKey,
    state: AsyncMutex<RoomState>,
    wake: Notify,
    closed: AtomicBool,
    next_conn: AtomicU64,
}

/// Farbe eines Users für die Cursoranzeige (deterministisch aus der User-ID).
fn color_for(user: Uuid) -> &'static str {
    const PALETTE: [&str; 10] = [
        "#e5484d", "#f76b15", "#ffb224", "#30a46c", "#0d9373", "#0091ff", "#3e63dd", "#8e4ec6",
        "#d6409f", "#978365",
    ];
    let h = user.as_u128();
    PALETTE[usize::try_from(h % PALETTE.len() as u128).unwrap_or(0)]
}

fn encode_msg(m: &YMessage) -> Vec<u8> {
    m.encode_v1()
}

// --- Raum laden -----------------------------------------------------------------------------

fn target_ctx_err(e: &AppError) -> StatusCode {
    match e.kind {
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::Forbidden => StatusCode::FORBIDDEN,
        ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// Baut das Dokument des Raums: gespeicherter Zustand, wenn er zum Markdown passt (sonst
/// Erstbefüllung aus Markdown auf dem Server, nie durch einen Client).
async fn build_room_doc(
    app: &App,
    ctx: &RequestCtx,
    target: CollabTarget,
    mut load: CollabLoad,
) -> Result<(Doc, Uuid, i32), AppError> {
    for _ in 0..3 {
        if let Some(state) = load.state.as_deref()
            && let Some(doc) = crdt::doc_from_state(state)
            && crdt::check_limits(&doc).is_ok()
            && crdt::state_matches_markdown(&doc, &load.markdown)
        {
            if let Some(epoch) = load.epoch {
                return Ok((doc, epoch, load.version));
            }
            // Zustand ohne Epoche (Altbestand): Epoche vergeben und festhalten.
            let epoch = Uuid::now_v7();
            if app
                .collab_seed(ctx, target, &crdt::state_to_b64(&doc), epoch, load.version)
                .await?
            {
                return Ok((doc, epoch, load.version));
            }
        } else {
            let doc = crdt::doc_from_markdown(&load.markdown);
            let epoch = Uuid::now_v7();
            if app
                .collab_seed(ctx, target, &crdt::state_to_b64(&doc), epoch, load.version)
                .await?
            {
                return Ok((doc, epoch, load.version));
            }
        }
        // Inhalt hat sich zwischenzeitlich geändert: neu laden.
        load = app.collab_load(ctx, target).await?;
    }
    Err(AppError::conflict("content keeps changing"))
}

impl Shared {
    /// Liefert den Raum, lädt ihn bei Bedarf (genau einmal, auch bei gleichzeitigem Beitritt).
    async fn room(
        self: &Arc<Self>,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> Result<Arc<Room>, AppError> {
        let key: RoomKey = (ctx.org_id.0, target);
        for _ in 0..4 {
            let cell = {
                let mut map = lock(&self.rooms.map);
                if !map.contains_key(&key) && map.len() >= self.cfg.collab.max_rooms {
                    return Err(AppError::rate_limited("Too many open documents."));
                }
                map.entry(key).or_default().clone()
            };
            let room = cell
                .get_or_try_init(|| async {
                    let load = self.app.collab_load(ctx, target).await?;
                    let (doc, epoch, version) =
                        build_room_doc(&self.app, ctx, target, load).await?;
                    // Altbestand: Kommentar-Anker aus `quotedText` ableiten (best effort).
                    if let Err(e) = self.app.collab_backfill_anchors(ctx, target, &doc).await {
                        tracing::debug!(error = %e, "collab anchor backfill failed");
                    }
                    let now = Instant::now();
                    let checkpoint = doc
                        .transact()
                        .encode_state_as_update_v1(&StateVector::default());
                    let approx = checkpoint.len();
                    let room = Arc::new(Room {
                        key,
                        state: AsyncMutex::new(RoomState {
                            doc,
                            epoch,
                            version,
                            checkpoint,
                            log: vec![],
                            log_bytes: 0,
                            approx_state_bytes: approx,
                            conns: HashMap::new(),
                            awareness: HashMap::new(),
                            generation: 0,
                            saved_generation: 0,
                            dirty_since: None,
                            last_change: now,
                            last_writer: None,
                            empty_since: Some(now),
                            persisting: false,
                        }),
                        wake: Notify::new(),
                        closed: AtomicBool::new(false),
                        next_conn: AtomicU64::new(1),
                    });
                    tokio::spawn(room_task(self.clone(), room.clone()));
                    Ok::<_, AppError>(room)
                })
                .await;
            match room {
                Ok(room) if !room.closed.load(Ordering::Acquire) => return Ok(room.clone()),
                Ok(_) => {
                    // Wird gerade entladen: Eintrag entfernen lassen und erneut versuchen.
                    tokio::task::yield_now().await;
                }
                Err(e) => {
                    // Fehlgeschlagene Initialisierung nicht im Verzeichnis stehen lassen.
                    let mut map = lock(&self.rooms.map);
                    if map.get(&key).is_some_and(|c| !c.initialized()) {
                        map.remove(&key);
                    }
                    return Err(e);
                }
            }
        }
        Err(AppError::conflict("Room is being closed, retry."))
    }

    fn acquire_collab(self: &Arc<Self>, user: Uuid) -> Option<CollabGuard> {
        let mut users = lock(&self.rooms.users);
        let n = users.entry(user).or_insert(0);
        if *n >= self.cfg.collab.max_connections_per_user {
            return None;
        }
        *n += 1;
        Some(CollabGuard {
            shared: self.clone(),
            user,
        })
    }
}

pub(crate) struct CollabGuard {
    shared: Arc<Shared>,
    user: Uuid,
}

impl Drop for CollabGuard {
    fn drop(&mut self) {
        let mut users = lock(&self.shared.rooms.users);
        if let Some(n) = users.get_mut(&self.user) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                users.remove(&self.user);
            }
        }
    }
}

// --- Raumaufgaben ---------------------------------------------------------------------------

/// Verteilt eine Nachricht an alle Teilnehmer außer `except`; zu langsame werden getrennt.
fn broadcast(st: &RoomState, except: Option<u64>, frame: &[u8]) {
    for (id, c) in &st.conns {
        if Some(*id) == except {
            continue;
        }
        if c.tx.try_send(Out::Frame(frame.to_vec())).is_err() {
            let _ = c.tx.try_send(Out::Close(1013, "too slow"));
        }
    }
}

fn close_all(st: &RoomState, code: u16, reason: &'static str) {
    for c in st.conns.values() {
        let _ = c.tx.try_send(Out::Close(code, reason));
    }
}

/// Baut das Dokument aus Prüfpunkt und akzeptierten Updates neu auf (Verwerfen eines Verstoßes).
fn rollback(st: &mut RoomState) {
    let doc = crdt::new_doc();
    let _ = doc.get_or_insert_xml_fragment(lineal_app::collab::pm::FRAGMENT);
    let mut ok = true;
    if let Ok(u) = Update::decode_v1(&st.checkpoint) {
        ok &= doc.transact_mut().apply_update(u).is_ok();
    }
    for bytes in &st.log {
        if let Ok(u) = Update::decode_v1(bytes) {
            ok &= doc.transact_mut().apply_update(u).is_ok();
        }
    }
    if !ok {
        tracing::error!("collab rollback replay failed");
    }
    st.doc = doc;
}

enum Applied {
    Ok,
    Invalid,
    TooBig,
}

/// Wendet ein Update an (unter `catch_unwind`), prüft Struktur- und Größengrenzen und rollt bei
/// Verstoß zurück. Ändert nichts an Verteilung/Persistenz.
fn apply_update(st: &mut RoomState, cfg: &CollabConfig, bytes: &[u8]) -> Applied {
    let Ok(update) = Update::decode_v1(bytes) else {
        return Applied::Invalid;
    };
    let applied = catch_unwind(AssertUnwindSafe(|| {
        st.doc.transact_mut().apply_update(update).is_ok()
    }));
    if !matches!(applied, Ok(true)) {
        rollback(st);
        return Applied::Invalid;
    }
    // Grenzen prüfen (Struktur immer, Größe über Schätzung, bei Überschreitung exakt).
    let structural = catch_unwind(AssertUnwindSafe(|| crdt::check_limits(&st.doc)));
    let structure_ok = matches!(structural, Ok(Ok(())));
    let mut too_big = false;
    if structure_ok {
        st.approx_state_bytes = st.approx_state_bytes.saturating_add(bytes.len());
        if st.approx_state_bytes > cfg.max_doc_state_bytes {
            st.approx_state_bytes = crdt::state_size(&st.doc);
            too_big = st.approx_state_bytes > cfg.max_doc_state_bytes;
        }
    }
    if !structure_ok || too_big {
        rollback(st);
        st.approx_state_bytes = crdt::state_size(&st.doc);
        return if structure_ok {
            Applied::TooBig
        } else {
            // Tiefe/Knotenzahl überschritten.
            Applied::TooBig
        };
    }
    st.log_bytes += bytes.len();
    st.log.push(bytes.to_vec());
    if st.log_bytes > 1024 * 1024 || st.log.len() > 2000 {
        st.checkpoint = st
            .doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        st.log.clear();
        st.log_bytes = 0;
    }
    Applied::Ok
}

fn mark_dirty(st: &mut RoomState, room: &Room) {
    let now = Instant::now();
    st.generation += 1;
    st.last_change = now;
    st.dirty_since.get_or_insert(now);
    room.wake.notify_one();
}

/// Arbeitet einen externen Stand (API/MCP/Wiederherstellung) in den Raum ein: gleiche CRDT-Linie
/// → Zustand zusammenführen; sonst per minimalem Markdown-Diff.
fn merge_external(st: &mut RoomState, room: &Room, load: &CollabLoad) {
    let diff = match (load.state.as_deref(), load.epoch) {
        (Some(state), Some(epoch)) if epoch == st.epoch => crdt::merge_state(&st.doc, state),
        _ => crdt::apply_markdown(&st.doc, &load.markdown),
    };
    st.version = load.version;
    if let Some(update) = diff {
        st.log_bytes += update.len();
        st.log.push(update.clone());
        st.approx_state_bytes = st.approx_state_bytes.saturating_add(update.len());
        broadcast(
            st,
            None,
            &encode_msg(&YMessage::Sync(SyncMessage::Update(update))),
        );
        // Zusammengeführter Stand (inkl. ungespeicherter Raumänderungen) muss zurückgeschrieben werden.
        mark_dirty(st, room);
    }
}

async fn room_task(shared: Arc<Shared>, room: Arc<Room>) {
    let cfg = shared.cfg.collab.clone();
    let mut tick = tokio::time::interval(cfg.poll_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = room.wake.notified() => {}
        }
        // Entscheidungen unter der Sperre, Datenbankarbeit außerhalb.
        enum Job {
            Persist(Box<PersistJob>),
            Poll(RequestCtx),
            Unload,
            Idle,
        }
        let job = {
            let mut st = room.state.lock().await;
            let now = Instant::now();
            let due = st.dirty_since.is_some_and(|since| {
                now.duration_since(st.last_change) >= cfg.persist_quiet
                    || now.duration_since(since) >= cfg.persist_max
            }) || (st.conns.is_empty() && st.dirty_since.is_some());
            if st.conns.is_empty() {
                st.empty_since.get_or_insert(now);
            } else {
                st.empty_since = None;
            }
            if due && !st.persisting {
                let ctx = st.last_writer.clone().or_else(|| {
                    st.conns
                        .values()
                        .find(|c| c.can_write)
                        .map(|c| c.ctx.clone())
                });
                match ctx {
                    Some(ctx) => {
                        st.persisting = true;
                        Job::Persist(Box::new(PersistJob {
                            ctx,
                            markdown: crdt::markdown_of(&st.doc),
                            state: crdt::state_to_b64(&st.doc),
                            epoch: st.epoch,
                            version: st.version,
                            generation: st.generation,
                        }))
                    }
                    None => Job::Idle,
                }
            } else if st.persisting {
                Job::Idle
            } else if let Some(c) = st.conns.values().next() {
                Job::Poll(c.ctx.clone())
            } else if st.dirty_since.is_none()
                && st
                    .empty_since
                    .is_some_and(|t| now.duration_since(t) >= cfg.idle_unload)
            {
                Job::Unload
            } else if st.dirty_since.is_some()
                && st
                    .empty_since
                    .is_some_and(|t| now.duration_since(t) >= cfg.idle_unload * 5)
            {
                // Ungespeicherte Änderungen ohne schreibberechtigten Kontext: aufgeben.
                tracing::warn!("collab room dropped with unsaved changes");
                Job::Unload
            } else {
                Job::Idle
            }
        };
        match job {
            Job::Idle => {}
            Job::Unload => {
                let st = room.state.lock().await;
                if st.conns.is_empty() {
                    room.closed.store(true, Ordering::Release);
                    drop(st);
                    lock(&shared.rooms.map).remove(&room.key);
                    return;
                }
            }
            Job::Poll(ctx) => poll_external(&shared, &room, &ctx).await,
            Job::Persist(job) => persist(&shared, &room, *job).await,
        }
    }
}

struct PersistJob {
    ctx: RequestCtx,
    markdown: String,
    state: String,
    epoch: Uuid,
    version: i32,
    generation: u64,
}

async fn persist(shared: &Arc<Shared>, room: &Arc<Room>, job: PersistJob) {
    let target = room.key.1;
    let result = shared
        .app
        .collab_persist(
            &job.ctx,
            target,
            &job.markdown,
            &job.state,
            job.epoch,
            job.version,
        )
        .await;
    let mut st = room.state.lock().await;
    st.persisting = false;
    match result {
        Ok(PersistOutcome::Saved { version } | PersistOutcome::Unchanged { version }) => {
            st.version = version;
            st.saved_generation = job.generation;
            if st.generation == job.generation {
                st.dirty_since = None;
            } else {
                st.dirty_since = Some(Instant::now());
            }
        }
        Ok(PersistOutcome::Stale(load)) => {
            merge_external(&mut st, room, &load);
            // Erneut speichern (dirty bleibt/wird gesetzt).
            st.dirty_since.get_or_insert_with(Instant::now);
            room.wake.notify_one();
        }
        Err(e) if e.kind == ErrorKind::Forbidden || e.kind == ErrorKind::NotFound => {
            // Der schreibende User darf nicht mehr: seine Verbindungen schließen, einen anderen
            // Schreiber beim nächsten Durchlauf verwenden.
            let user = match job.ctx.actor {
                Actor::User { id, .. } | Actor::App { id } => Some(id.0),
                Actor::System => None,
            };
            st.last_writer = None;
            for c in st.conns.values() {
                if Some(c.user) == user {
                    let _ = c.tx.try_send(Out::Close(CLOSE_FORBIDDEN, "forbidden"));
                }
            }
            if e.kind == ErrorKind::NotFound {
                close_all(&st, CLOSE_GONE, "gone");
            }
        }
        Err(e) if e.kind == ErrorKind::LimitExceeded || e.kind == ErrorKind::InvalidRequest => {
            // Inhalt zu groß: Raum einfrieren (Schreiber trennen), Stand bleibt im Speicher.
            tracing::warn!(error = %e, "collab persist rejected content");
            for c in st.conns.values().filter(|c| c.can_write) {
                let _ =
                    c.tx.try_send(Out::Close(CLOSE_TOO_BIG, "content too large"));
            }
            st.dirty_since = None;
        }
        Err(e) => {
            tracing::warn!(error = %e, "collab persist failed, will retry");
            st.dirty_since.get_or_insert_with(Instant::now);
        }
    }
}

async fn poll_external(shared: &Arc<Shared>, room: &Arc<Room>, ctx: &RequestCtx) {
    let target = room.key.1;
    match shared.app.collab_version(ctx, target).await {
        Ok(v) => {
            let current = room.state.lock().await.version;
            if v != current {
                match shared.app.collab_load(ctx, target).await {
                    Ok(load) => {
                        let mut st = room.state.lock().await;
                        if !st.persisting && load.version != st.version {
                            merge_external(&mut st, room, &load);
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "collab external load failed"),
                }
            }
        }
        Err(e) if e.kind == ErrorKind::NotFound => {
            let st = room.state.lock().await;
            close_all(&st, CLOSE_GONE, "gone");
        }
        Err(_) => {}
    }
}

// --- Endpunkt -------------------------------------------------------------------------------

pub(crate) async fn collab_socket(
    State(shared): State<Arc<Shared>>,
    Path((kind, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    // Reihenfolge: erst Auth (401), dann Upgrade-Fehler, Origin, Scope.
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
    let target = match kind.as_str() {
        "document" => CollabTarget::Document(DocumentId(id)),
        "issue" => CollabTarget::Issue(IssueId(id)),
        "project" => CollabTarget::Project(ProjectId(id)),
        "initiative" => CollabTarget::Initiative(InitiativeId(id)),
        _ => return error_response(StatusCode::NOT_FOUND, "not_found", "Unknown content kind."),
    };
    // Sichtbarkeit und Recht: unsichtbar = nicht vorhanden (404), vor dem Upgrade.
    let access = match shared.app.collab_open(&ctx, target).await {
        Ok(a) => a,
        Err(e) if e.kind == ErrorKind::Internal => return app_error_response(&e),
        Err(e) => {
            let status = target_ctx_err(&e);
            return error_response(status, e.kind.code(), &e.message);
        }
    };
    let Some(guard) = shared.acquire_collab(user) else {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many collaboration connections for this user.",
        );
    };
    shared.ensure_listener();
    let max = shared.cfg.collab.max_update_bytes + 1024;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| {
            collab_connection(socket, shared, (ctx, token_hash), target, access, guard)
        })
}

async fn send_msg(socket: &mut WebSocket, cfg: &CollabConfig, msg: Message) -> Result<(), End> {
    match timeout(cfg.send_timeout, socket.send(msg)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(End::Closed),
        Err(_) => Err(End::Slow),
    }
}

async fn close_with(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

/// Token-Bucket je Verbindung.
struct Bucket {
    tokens: f64,
    burst: f64,
    rate: f64,
    last: Instant,
}

impl Bucket {
    fn new(rate: u32, burst: u32) -> Self {
        Self {
            tokens: f64::from(burst),
            burst: f64::from(burst),
            rate: f64::from(rate),
            last: Instant::now(),
        }
    }
    fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens =
            (self.tokens + now.duration_since(self.last).as_secs_f64() * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn collab_connection(
    mut socket: WebSocket,
    shared: Arc<Shared>,
    (mut ctx, token_hash): (RequestCtx, String),
    target: CollabTarget,
    mut access: CollabAccess,
    _guard: CollabGuard,
) {
    let cfg = shared.cfg.collab.clone();
    let user = match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => id.0,
        Actor::System => return,
    };
    let room = match shared.room(&ctx, target).await {
        Ok(r) => r,
        Err(e) => {
            tracing::info!(error = %e, "collab join failed");
            let code = match e.kind {
                ErrorKind::NotFound => CLOSE_GONE,
                ErrorKind::Forbidden => CLOSE_FORBIDDEN,
                ErrorKind::RateLimited => 1013,
                _ => 1011,
            };
            close_with(&mut socket, code, "join failed").await;
            return;
        }
    };
    let (tx, mut rx) = mpsc::channel::<Out>(cfg.outbound_buffer);
    let conn_id = room.next_conn.fetch_add(1, Ordering::Relaxed);
    // Beitritt + Hello + Schritt 1 + Awareness atomar unter der Raumsperre (nichts geht verloren).
    {
        let mut st = room.state.lock().await;
        if st.conns.len() >= cfg.max_connections_per_room {
            drop(st);
            close_with(&mut socket, 1013, "room full").await;
            return;
        }
        let hello = json!({
            "type": "hello",
            "epoch": st.epoch,
            "canWrite": access.can_write,
            "version": st.version,
            "user": { "name": access.user_name, "color": color_for(user) },
        });
        let step1 = encode_msg(&YMessage::Sync(SyncMessage::SyncStep1(
            st.doc.transact().state_vector(),
        )));
        let aw = awareness_snapshot(&st);
        st.conns.insert(
            conn_id,
            ConnEntry {
                tx: tx.clone(),
                user,
                ctx: ctx.clone(),
                can_write: access.can_write,
                client_ids: HashSet::new(),
            },
        );
        st.empty_since = None;
        let _ = tx.try_send(Out::Text(hello.to_string()));
        let _ = tx.try_send(Out::Frame(step1));
        if let Some(aw) = aw {
            let _ = tx.try_send(Out::Frame(aw));
        }
    }
    let end = drive_collab(
        &mut socket,
        &shared,
        &room,
        conn_id,
        &mut rx,
        &mut ctx,
        &token_hash,
        target,
        &mut access,
    )
    .await;
    // Austritt: Awareness entfernen, ggf. sofort speichern lassen.
    {
        let mut st = room.state.lock().await;
        let removed: Vec<u64> = st
            .conns
            .remove(&conn_id)
            .map(|c| c.client_ids.into_iter().collect())
            .unwrap_or_default();
        let mut entries = HashMap::new();
        for cid in removed {
            if let Some(slot) = st.awareness.remove(&cid) {
                entries.insert(
                    yrs_client(cid),
                    AwarenessUpdateEntry {
                        clock: slot.clock.saturating_add(1),
                        json: "null".into(),
                    },
                );
            }
        }
        if !entries.is_empty() {
            broadcast(
                &st,
                None,
                &encode_msg(&YMessage::Awareness(AwarenessUpdate { clients: entries })),
            );
        }
        if st.conns.is_empty() {
            st.empty_since = Some(Instant::now());
        }
        room.wake.notify_one();
    }
    match end {
        End::Closed => {}
        End::Slow => tracing::info!("collab client too slow"),
        End::Idle => close_with(&mut socket, 1001, "idle timeout").await,
        End::Unauthorized => close_with(&mut socket, CLOSE_UNAUTHORIZED, "unauthorized").await,
        End::Error(e) => {
            tracing::warn!(error = %e, "collab connection ended with error");
            close_with(&mut socket, 1011, "collab ended").await;
        }
        End::Code(code, reason) => close_with(&mut socket, code, reason).await,
    }
}

fn yrs_client(id: u64) -> yrs::block::ClientID {
    id
}

fn awareness_snapshot(st: &RoomState) -> Option<Vec<u8>> {
    if st.awareness.is_empty() {
        return None;
    }
    let clients = st
        .awareness
        .iter()
        .map(|(id, s)| {
            (
                yrs_client(*id),
                AwarenessUpdateEntry {
                    clock: s.clock,
                    json: s.json.clone(),
                },
            )
        })
        .collect();
    Some(encode_msg(&YMessage::Awareness(AwarenessUpdate {
        clients,
    })))
}

/// Schreibt die Awareness eines Clients auf die authentifizierte Identität um. `None` = ungültig.
fn sanitize_awareness(json: &str, name: &str, user: Uuid) -> Option<Arc<str>> {
    if json == "null" {
        return Some("null".into());
    }
    if json.len() > 4096 {
        return None;
    }
    let v: Value = serde_json::from_str(json).ok()?;
    let obj = v.as_object()?;
    let mut out = serde_json::Map::new();
    out.insert(
        "user".into(),
        json!({ "name": name, "color": color_for(user), "id": user }),
    );
    // Nur der Cursor des Editors wird übernommen; alles andere ist Sache des Servers.
    if let Some(c) = obj.get("cursor") {
        // Relative Positionen: genau `anchor`/`head`, je klein und ein Objekt (keine Nutzlast).
        // `null` = kein Cursor (Editor ohne Fokus).
        if c.is_null() {
            out.insert("cursor".into(), Value::Null);
        } else {
            let cursor = c.as_object()?;
            let mut clean = serde_json::Map::new();
            for key in ["anchor", "head"] {
                if let Some(p) = cursor.get(key) {
                    if !p.is_object() || p.to_string().len() > 512 {
                        return None;
                    }
                    clean.insert(key.into(), p.clone());
                }
            }
            out.insert("cursor".into(), Value::Object(clean));
        }
    }
    Some(Value::Object(out).to_string().into())
}

enum Handled {
    Continue,
    Stop(End),
}

#[allow(clippy::too_many_arguments)]
async fn drive_collab(
    socket: &mut WebSocket,
    shared: &Arc<Shared>,
    room: &Arc<Room>,
    conn_id: u64,
    rx: &mut mpsc::Receiver<Out>,
    ctx: &mut RequestCtx,
    token_hash: &str,
    target: CollabTarget,
    access: &mut CollabAccess,
) -> End {
    let cfg = shared.cfg.collab.clone();
    let mut wake = shared.wake.subscribe();
    let start = Instant::now();
    let mut ping = interval_at(start + cfg.ping_interval, cfg.ping_interval);
    let mut recheck = interval_at(start + cfg.revalidate_interval, cfg.revalidate_interval);
    let mut last_seen = Instant::now();
    let mut last_check = Instant::now();
    let mut pending_check = false;
    let mut throttle = interval_at(start + cfg.recheck_throttle, cfg.recheck_throttle);
    let mut bucket = Bucket::new(cfg.messages_per_sec, cfg.message_burst);
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(out) = out else { return End::Closed };
                let res = match out {
                    Out::Frame(b) => send_msg(socket, &cfg, Message::Binary(b.into())).await,
                    Out::Text(t) => send_msg(socket, &cfg, Message::Text(t.into())).await,
                    Out::Close(code, reason) => return End::Code(code, reason),
                };
                if let Err(e) = res { return e; }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return End::Closed };
                last_seen = Instant::now();
                match msg {
                    Message::Binary(data) => {
                        if !bucket.take() {
                            return End::Code(CLOSE_RATE_LIMITED, "rate limited");
                        }
                        if data.len() > cfg.max_update_bytes + 1024 {
                            return End::Code(CLOSE_TOO_BIG, "message too large");
                        }
                        match handle_binary(shared, room, conn_id, ctx, access, &data).await {
                            Handled::Continue => {}
                            Handled::Stop(e) => return e,
                        }
                    }
                    Message::Text(text) => {
                        if !bucket.take() {
                            return End::Code(CLOSE_RATE_LIMITED, "rate limited");
                        }
                        if serde_json::from_str::<Value>(&text).is_ok_and(|v| v["type"] == "ping") {
                            let r = send_msg(socket, &cfg, Message::Text(json!({"type":"pong"}).to_string().into())).await;
                            if let Err(e) = r { return e; }
                        }
                    }
                    Message::Close(_) => return End::Closed,
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            r = wake.recv() => {
                match r {
                    // Mitgliedschaften/Teams können sich geändert haben: bald neu prüfen.
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => pending_check = true,
                    Err(broadcast::error::RecvError::Closed) => {}
                }
            }
            _ = throttle.tick() => {
                if pending_check && last_check.elapsed() >= cfg.recheck_throttle {
                    pending_check = false;
                    last_check = Instant::now();
                    if let Some(e) = recheck_access(shared, room, conn_id, ctx, token_hash, target, access).await {
                        return e;
                    }
                }
            }
            _ = recheck.tick() => {
                last_check = Instant::now();
                if let Some(e) = recheck_access(shared, room, conn_id, ctx, token_hash, target, access).await {
                    return e;
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() > cfg.idle_timeout {
                    return End::Idle;
                }
                if let Err(e) = send_msg(socket, &cfg, Message::Ping(Vec::new().into())).await {
                    return e;
                }
            }
        }
    }
}

/// Prüft Token und Rechte neu. `Some(End)` = Verbindung beenden.
async fn recheck_access(
    shared: &Arc<Shared>,
    room: &Arc<Room>,
    conn_id: u64,
    ctx: &mut RequestCtx,
    token_hash: &str,
    target: CollabTarget,
    access: &mut CollabAccess,
) -> Option<End> {
    if let Err(e) = revalidate(shared, token_hash, ctx).await {
        return Some(e);
    }
    match shared.app.collab_open(ctx, target).await {
        Ok(a) => {
            let lost_write = access.can_write && !a.can_write;
            if lost_write {
                return Some(End::Code(CLOSE_FORBIDDEN, "forbidden"));
            }
            // Gewonnene Rechte gelten nach dem nächsten Verbindungsaufbau (Hello trägt canWrite).
            let mut st = room.state.lock().await;
            if let Some(c) = st.conns.get_mut(&conn_id) {
                c.ctx = ctx.clone();
            }
            None
        }
        Err(e) if e.kind == ErrorKind::NotFound => Some(End::Code(CLOSE_GONE, "gone")),
        Err(e) if e.kind == ErrorKind::Forbidden => Some(End::Code(CLOSE_FORBIDDEN, "forbidden")),
        Err(e) => {
            tracing::debug!(error = %e, "collab recheck failed");
            None
        }
    }
}

async fn handle_binary(
    shared: &Arc<Shared>,
    room: &Arc<Room>,
    conn_id: u64,
    ctx: &RequestCtx,
    access: &CollabAccess,
    data: &[u8],
) -> Handled {
    let cfg = &shared.cfg.collab;
    let Ok(msg) = YMessage::decode_v1(data) else {
        return Handled::Stop(End::Code(CLOSE_INVALID, "invalid message"));
    };
    match msg {
        YMessage::Sync(SyncMessage::SyncStep1(sv)) => {
            let st = room.state.lock().await;
            let update = st.doc.transact().encode_state_as_update_v1(&sv);
            if let Some(c) = st.conns.get(&conn_id) {
                let _ = c.tx.try_send(Out::Frame(encode_msg(&YMessage::Sync(
                    SyncMessage::SyncStep2(update),
                ))));
            }
            Handled::Continue
        }
        YMessage::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) => {
            if !access.can_write {
                // Leere Antworten (Schritt 2 ohne Inhalt, 2 Bytes) sind für Nur-Leser normal.
                if update.len() <= 2 {
                    return Handled::Continue;
                }
                // Nur-Leser dürfen keine Änderungen senden: verwerfen und trennen.
                return Handled::Stop(End::Code(CLOSE_FORBIDDEN, "read only"));
            }
            if update.len() > cfg.max_update_bytes {
                return Handled::Stop(End::Code(CLOSE_TOO_BIG, "update too large"));
            }
            // Leere Updates (2 Bytes) sind Schritt-2-Antworten ohne Inhalt.
            let mut st = room.state.lock().await;
            match apply_update(&mut st, cfg, &update) {
                Applied::Ok => {}
                Applied::Invalid => {
                    return Handled::Stop(End::Code(CLOSE_INVALID, "invalid update"));
                }
                Applied::TooBig => {
                    return Handled::Stop(End::Code(CLOSE_TOO_BIG, "document too large"));
                }
            }
            if update.len() > 2 {
                broadcast(
                    &st,
                    Some(conn_id),
                    &encode_msg(&YMessage::Sync(SyncMessage::Update(update))),
                );
                st.last_writer = Some(ctx.clone());
                mark_dirty(&mut st, room);
            }
            Handled::Continue
        }
        YMessage::AwarenessQuery => {
            let st = room.state.lock().await;
            if let (Some(aw), Some(c)) = (awareness_snapshot(&st), st.conns.get(&conn_id)) {
                let _ = c.tx.try_send(Out::Frame(aw));
            }
            Handled::Continue
        }
        YMessage::Awareness(update) => {
            let user = match ctx.actor {
                Actor::User { id, .. } | Actor::App { id } => id.0,
                Actor::System => return Handled::Stop(End::Closed),
            };
            if update.clients.len() > 4 {
                return Handled::Stop(End::Code(CLOSE_INVALID, "invalid awareness"));
            }
            let mut st = room.state.lock().await;
            let mut accepted = HashMap::new();
            for (cid, entry) in update.clients {
                let id = cid;
                // Client-IDs gehören genau einer Verbindung (kein Überschreiben fremder Cursor).
                let mut owner = st.awareness.get(&id).map(|s| s.conn);
                if let Some(old) = owner.filter(|o| *o != conn_id) {
                    // Derselbe User darf die ID einer (halboffenen) alten Verbindung übernehmen:
                    // der Browser behält die Client-ID beim Reconnect, der Server bemerkt die
                    // tote Verbindung aber erst nach dem Leerlauf-Timeout (B2.07).
                    let same_user = st.conns.get(&old).is_none_or(|c| c.user == user);
                    if !same_user {
                        return Handled::Stop(End::Code(CLOSE_INVALID, "invalid awareness"));
                    }
                    if let Some(c) = st.conns.get_mut(&old) {
                        c.client_ids.remove(&id);
                    }
                    st.awareness.remove(&id);
                    owner = None;
                }
                let Some(json) = sanitize_awareness(&entry.json, &access.user_name, user) else {
                    return Handled::Stop(End::Code(CLOSE_INVALID, "invalid awareness"));
                };
                let owned = st.conns.get(&conn_id).map_or(0, |c| c.client_ids.len());
                if owner.is_none() && owned >= 4 {
                    return Handled::Stop(End::Code(CLOSE_INVALID, "invalid awareness"));
                }
                if &*json == "null" {
                    st.awareness.remove(&id);
                    if let Some(c) = st.conns.get_mut(&conn_id) {
                        c.client_ids.remove(&id);
                    }
                } else {
                    st.awareness.insert(
                        id,
                        AwarenessSlot {
                            clock: entry.clock,
                            json: json.clone(),
                            conn: conn_id,
                        },
                    );
                    if let Some(c) = st.conns.get_mut(&conn_id) {
                        c.client_ids.insert(id);
                    }
                }
                accepted.insert(
                    cid,
                    AwarenessUpdateEntry {
                        clock: entry.clock,
                        json,
                    },
                );
            }
            if !accepted.is_empty() {
                broadcast(
                    &st,
                    Some(conn_id),
                    &encode_msg(&YMessage::Awareness(AwarenessUpdate { clients: accepted })),
                );
            }
            Handled::Continue
        }
        YMessage::Auth(_) | YMessage::Custom(..) => {
            Handled::Stop(End::Code(CLOSE_INVALID, "unsupported message"))
        }
    }
}
