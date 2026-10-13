/**
 * Provider der Echtzeit-Mitbearbeitung (docs/06 §1.9 Stufe 2) gegen `GET /collab/document/{id}`.
 *
 * Protokoll: erst ein Textframe `hello` (Epoche, Schreibrecht, eigener Anzeigename/Farbe), danach
 * binäre y-protocols-Frames (Sync 1/2/Update, Awareness). Eigenschaften:
 *
 * - Reconnect mit exponentiellem Backoff (mit Jitter). Lokale Änderungen bleiben im `Y.Doc`
 *   (Offline-Puffer) und werden nach dem Wiederverbinden über Sync-Schritt 1/2 nachgereicht.
 * - Ändert sich die Epoche (der Server hat die CRDT-Linie neu aufgebaut), meldet der Provider
 *   `reset`: der lokale Stand darf nicht mit dem neuen zusammengeführt werden (Verdopplung).
 * - Nie erreichbar (z. B. Endpunkt fehlt, 404/401 beim Upgrade, nicht mehr als `maxInitialFailures`
 *   Fehlversuche vor der ersten Verbindung): Status `fallback` → die Oberfläche speichert wie in
 *   Stufe 1 (Autosave mit `expectedVersion`).
 * - Nur-Lese-Modus: der Server meldet `canWrite=false`; es werden keine Änderungen gesendet.
 */
import * as decoding from "lib0/decoding";
import * as encoding from "lib0/encoding";
import * as awarenessProtocol from "y-protocols/awareness";
import * as syncProtocol from "y-protocols/sync";
import * as Y from "yjs";

const MSG_SYNC = 0;
const MSG_AWARENESS = 1;
const MSG_QUERY_AWARENESS = 3;

/** Schließcodes des Servers (docs/15). */
export const CLOSE_UNAUTHORIZED = 4401;
export const CLOSE_FORBIDDEN = 4403;
export const CLOSE_GONE = 4404;

export type CollabStatus = "connecting" | "connected" | "disconnected" | "fallback" | "closed";

export interface CollabIdentity {
  name: string;
  color: string;
}

export interface CollabOptions {
  /** Vollständige `ws(s)://`-URL. */
  url: string;
  WebSocketImpl?: typeof WebSocket;
  /** Fehlversuche vor der ersten Verbindung, nach denen auf Stufe 1 zurückgefallen wird. */
  maxInitialFailures?: number;
  baseDelayMs?: number;
  maxDelayMs?: number;
  pingMs?: number;
  /** So lange muss eine Verbindung halten, bevor der Backoff zurückgesetzt wird. */
  stableMs?: number;
  random?: () => number;
}

type Listener = () => void;

export class CollabProvider {
  readonly doc = new Y.Doc();
  readonly awareness = new awarenessProtocol.Awareness(this.doc);
  status: CollabStatus = "connecting";
  canWrite = false;
  synced = false;
  epoch: string | null = null;
  identity: CollabIdentity | null = null;
  /** Grund des Status `closed`. */
  closeCode: number | null = null;

  private ws: WebSocket | null = null;
  private attempts = 0;
  /** Schon einmal verbunden gewesen (danach gilt Offline-Betrieb statt Fallback). */
  everConnected = false;
  private helloSeen = false;
  private destroyed = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private stableTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private readonly listeners = new Set<Listener>();
  private readonly resetListeners = new Set<Listener>();
  private readonly opts: Required<Omit<CollabOptions, "WebSocketImpl">> & {
    WebSocketImpl: typeof WebSocket;
  };

  constructor(options: CollabOptions) {
    this.opts = {
      url: options.url,
      WebSocketImpl: options.WebSocketImpl ?? WebSocket,
      maxInitialFailures: options.maxInitialFailures ?? 3,
      baseDelayMs: options.baseDelayMs ?? 500,
      maxDelayMs: options.maxDelayMs ?? 30_000,
      pingMs: options.pingMs ?? 25_000,
      stableMs: options.stableMs ?? 10_000,
      random: options.random ?? Math.random,
    };
    this.doc.on("update", this.onDocUpdate);
    this.awareness.on("update", this.onAwarenessUpdate);
  }

  /** Änderungen von Status/Schreibrecht/Synchronisation. */
  subscribe(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  /** Die Epoche hat gewechselt: ein neuer Provider mit leerem Dokument ist nötig. */
  onReset(l: Listener): () => void {
    this.resetListeners.add(l);
    return () => this.resetListeners.delete(l);
  }

  connect(): void {
    if (this.destroyed || this.ws) return;
    this.setStatus(this.everConnected ? "disconnected" : "connecting");
    let ws: WebSocket;
    try {
      ws = new this.opts.WebSocketImpl(this.opts.url);
    } catch {
      this.onClosed(1006);
      return;
    }
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    ws.onmessage = (ev) => this.onMessage(ev);
    ws.onclose = (ev) => {
      if (this.ws === ws) this.onClosed(ev.code);
    };
    ws.onerror = () => {
      // Das folgende `close` löst Reconnect/Fallback aus.
    };
  }

  destroy(): void {
    this.destroyed = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.clearStable();
    this.stopPing();
    const ws = this.ws;
    this.ws = null;
    ws?.close();
    this.doc.off("update", this.onDocUpdate);
    this.awareness.off("update", this.onAwarenessUpdate);
    awarenessProtocol.removeAwarenessStates(this.awareness, [this.doc.clientID], "destroy");
    this.awareness.destroy();
    this.doc.destroy();
    this.listeners.clear();
    this.resetListeners.clear();
  }

  // --- intern -----------------------------------------------------------------------

  private setStatus(s: CollabStatus): void {
    if (this.status === s) return;
    this.status = s;
    this.emit();
  }

  private emit(): void {
    for (const l of [...this.listeners]) l();
  }

  private get open(): boolean {
    return this.ws?.readyState === 1 && this.helloSeen;
  }

  private sendBinary(encoder: encoding.Encoder): void {
    if (this.ws?.readyState === 1) this.ws.send(encoding.toUint8Array(encoder));
  }

  private readonly onDocUpdate = (update: Uint8Array, origin: unknown): void => {
    // Nur eigene Änderungen senden (nicht vom Server angewendete) und nur mit Schreibrecht.
    if (origin === this || !this.open || !this.canWrite) return;
    const enc = encoding.createEncoder();
    encoding.writeVarUint(enc, MSG_SYNC);
    syncProtocol.writeUpdate(enc, update);
    this.sendBinary(enc);
  };

  private readonly onAwarenessUpdate = (
    { added, updated, removed }: { added: number[]; updated: number[]; removed: number[] },
    origin: unknown,
  ): void => {
    if (origin === this || !this.open) return;
    const changed = added.concat(updated, removed).filter((id) => id === this.doc.clientID);
    if (changed.length === 0) return;
    const enc = encoding.createEncoder();
    encoding.writeVarUint(enc, MSG_AWARENESS);
    encoding.writeVarUint8Array(
      enc,
      awarenessProtocol.encodeAwarenessUpdate(this.awareness, changed),
    );
    this.sendBinary(enc);
  };

  private onMessage(ev: MessageEvent): void {
    if (typeof ev.data === "string") {
      this.onText(ev.data);
      return;
    }
    if (!this.helloSeen) return;
    const decoder = decoding.createDecoder(new Uint8Array(ev.data as ArrayBuffer));
    const type = decoding.readVarUint(decoder);
    try {
      if (type === MSG_SYNC) {
        const enc = encoding.createEncoder();
        encoding.writeVarUint(enc, MSG_SYNC);
        const sub = syncProtocol.readSyncMessage(decoder, enc, this.doc, this);
        if (sub === syncProtocol.messageYjsSyncStep2 && !this.synced) {
          this.synced = true;
          this.emit();
        }
        // Antwort (Schritt 2 auf Schritt 1). Ein Nur-Leser hat nichts Eigenes zu liefern.
        if (encoding.length(enc) > 1 && this.canWrite) this.sendBinary(enc);
      } else if (type === MSG_AWARENESS) {
        awarenessProtocol.applyAwarenessUpdate(
          this.awareness,
          decoding.readVarUint8Array(decoder),
          this,
        );
        this.emit();
      } else if (type === MSG_QUERY_AWARENESS) {
        const enc = encoding.createEncoder();
        encoding.writeVarUint(enc, MSG_AWARENESS);
        encoding.writeVarUint8Array(
          enc,
          awarenessProtocol.encodeAwarenessUpdate(
            this.awareness,
            Array.from(this.awareness.getStates().keys()),
          ),
        );
        this.sendBinary(enc);
      }
    } catch {
      // Defekter Frame: ignorieren, der Server schließt bei echten Protokollfehlern.
    }
  }

  private onText(text: string): void {
    let msg: {
      type?: string;
      epoch?: string;
      canWrite?: boolean;
      user?: CollabIdentity;
    };
    try {
      msg = JSON.parse(text) as typeof msg;
    } catch {
      return;
    }
    if (msg.type !== "hello") return;
    if (this.epoch !== null && msg.epoch !== this.epoch) {
      // Neue CRDT-Linie: lokalen Stand verwerfen lassen (Provider-Neuaufbau durch den Aufrufer).
      this.stopPing();
      const ws = this.ws;
      this.ws = null;
      ws?.close();
      for (const l of [...this.resetListeners]) l();
      return;
    }
    this.epoch = msg.epoch ?? null;
    this.canWrite = msg.canWrite === true;
    this.identity = msg.user ?? null;
    this.helloSeen = true;
    this.everConnected = true;
    // Den Backoff erst nach einer stabilen Verbindung zurücksetzen: lehnt der Server nach dem
    // Handshake ab (z. B. 4400), darf das keine Reconnect-Schleife im Takt der Basiswartezeit ergeben.
    this.clearStable();
    this.stableTimer = setTimeout(() => {
      this.stableTimer = null;
      this.attempts = 0;
    }, this.opts.stableMs);
    this.synced = false;
    if (this.identity) {
      this.awareness.setLocalStateField("user", this.identity);
    }
    this.setStatus("connected");
    this.emit();
    // Sync-Schritt 1 + Awareness; der Server antwortet mit Schritt 2 und schickt eigenen Schritt 1,
    // womit lokale (zwischenzeitlich entstandene) Änderungen nachgereicht werden.
    const enc = encoding.createEncoder();
    encoding.writeVarUint(enc, MSG_SYNC);
    syncProtocol.writeSyncStep1(enc, this.doc);
    this.sendBinary(enc);
    if (this.awareness.getLocalState() !== null) {
      const a = encoding.createEncoder();
      encoding.writeVarUint(a, MSG_AWARENESS);
      encoding.writeVarUint8Array(
        a,
        awarenessProtocol.encodeAwarenessUpdate(this.awareness, [this.doc.clientID]),
      );
      this.sendBinary(a);
    }
    this.startPing();
  }

  private startPing(): void {
    this.stopPing();
    this.pingTimer = setInterval(() => {
      if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ type: "ping" }));
    }, this.opts.pingMs);
  }

  private stopPing(): void {
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = null;
  }

  private clearStable(): void {
    if (this.stableTimer) clearTimeout(this.stableTimer);
    this.stableTimer = null;
  }

  private onClosed(code: number): void {
    this.clearStable();
    this.ws = null;
    this.helloSeen = false;
    this.synced = false;
    this.stopPing();
    if (this.destroyed) return;
    // Andere Teilnehmer sind nach dem Verbindungsverlust nicht mehr sichtbar.
    const others = Array.from(this.awareness.getStates().keys()).filter(
      (id) => id !== this.doc.clientID,
    );
    awarenessProtocol.removeAwarenessStates(this.awareness, others, "disconnect");
    if (code === CLOSE_UNAUTHORIZED || code === CLOSE_GONE) {
      this.closeCode = code;
      this.setStatus("closed");
      return;
    }
    if (!this.everConnected) {
      this.attempts += 1;
      if (this.attempts >= this.opts.maxInitialFailures) {
        this.setStatus("fallback");
        return;
      }
    } else {
      this.attempts += 1;
    }
    this.setStatus("disconnected");
    const exp = Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** (this.attempts - 1));
    const delay = Math.round(exp * (0.5 + this.opts.random() * 0.5));
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }
}

export type CollabKind = "document" | "issue" | "project" | "initiative";

/** `ws(s)://host/collab/<kind>/<id>` passend zur aktuellen Seite. */
export function collabUrl(kind: CollabKind, id: string, loc: Location = window.location): string {
  const scheme = loc.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${loc.host}/collab/${kind}/${encodeURIComponent(id)}`;
}
