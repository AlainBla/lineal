/**
 * WebSocket-Client des Echtzeit-Syncs (`GET /sync`, docs/09 §5.3).
 *
 * - Verbindet mit `?since=<lastSyncId>`; der Server liefert zuerst die Lücke, dann live.
 * - `lastSyncId` wird nach jedem Paket persistiert (localStorage, je Workspace und User) und
 *   überlebt Reloads; ohne gespeicherte ID verbindet der Client live und lädt einmalig alles neu.
 * - Reconnect mit exponentiellem Backoff (Jitter); `resetRequired` verwirft die gespeicherte ID
 *   und lädt alle Caches neu.
 * - App-Ping, damit halb offene Verbindungen auffallen (Browser können keine WS-Pings senden).
 *
 * Mutationen laufen weiter über GraphQL; Antworten tragen kein `lastSyncId` (siehe Entscheidung
 * im WP15-Bericht), der Abgleich geschieht ausschließlich über diesen Strom.
 */
import type { QueryClient } from "@tanstack/react-query";

import { applySyncActions, type SyncAction } from "./sync-actions";

export type ServerMessage =
  | { type: "hello"; lastSyncId: number; resumed: boolean }
  | { type: "sync"; actions: SyncAction[]; lastSyncId: number }
  | { type: "pong"; lastSyncId: number }
  | { type: "resetRequired" };

export interface SyncStorage {
  get(): number | null;
  set(id: number): void;
  clear(): void;
}

/** Speicher für die `lastSyncId` in `localStorage` (Fehler, z. B. Private Mode, werden geschluckt). */
export function localStorageSync(scope: string, storage?: Storage): SyncStorage {
  const key = `lineal.sync.lastSyncId.${scope}`;
  const store = () => storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
  return {
    get() {
      try {
        const raw = store()?.getItem(key);
        const n = raw ? Number(raw) : NaN;
        return Number.isSafeInteger(n) && n >= 0 ? n : null;
      } catch {
        return null;
      }
    },
    set(id) {
      try {
        store()?.setItem(key, String(id));
      } catch {
        /* nicht kritisch: ohne gespeicherte ID wird beim nächsten Start neu geladen */
      }
    },
    clear() {
      try {
        store()?.removeItem(key);
      } catch {
        /* siehe oben */
      }
    },
  };
}

/** Teilmenge der WebSocket-API, die der Client braucht (für Tests ersetzbar). */
export interface SocketLike {
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: { code: number }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export type SyncStatus = "connecting" | "open" | "closed";

export interface SyncClientOptions {
  qc: QueryClient;
  storage: SyncStorage;
  /** Basis-URL des Endpunkts ohne Query, z. B. `wss://host/sync`. */
  url: string;
  createSocket?: (url: string) => SocketLike;
  /** Backoff: Start und Obergrenze in ms. */
  backoffMs?: { base: number; max: number };
  /** Abstand der App-Pings in ms. */
  pingMs?: number;
  /** Ohne Nachricht in dieser Zeit gilt die Verbindung als tot. */
  deadAfterMs?: number;
  random?: () => number;
  onStatus?: (status: SyncStatus) => void;
  /**
   * Erstbefüllung beim ersten Start (ohne gespeicherte `lastSyncId`): lädt den Bestand über
   * `/sync/bootstrap`, befüllt die Caches und liefert die `lastSyncId`, ab der der Strom läuft.
   * Schlägt sie fehl oder liefert `null`, gilt der bisherige Ablauf (alles neu laden).
   */
  bootstrap?: () => Promise<number | null>;
}

const MAX_ACTIONS_PER_MESSAGE = 5000;

export class SyncClient {
  private socket: SocketLike | null = null;
  private attempts = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private pingTimer: ReturnType<typeof setInterval> | undefined;
  private lastMessageAt = 0;
  private stopped = true;
  private resumed = false;

  constructor(private readonly o: SyncClientOptions) {}

  /** Zuletzt gespeicherte Stream-ID. */
  get lastSyncId(): number | null {
    return this.o.storage.get();
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    if (this.o.bootstrap && this.o.storage.get() === null) {
      void this.bootstrapThenConnect(this.o.bootstrap);
    } else {
      this.connect();
    }
  }

  private async bootstrapThenConnect(bootstrap: () => Promise<number | null>): Promise<void> {
    try {
      const id = await bootstrap();
      if (this.stopped) return;
      if (id !== null && Number.isSafeInteger(id) && id >= 0) {
        this.o.storage.set(id);
        // Anfragen, die vor dem Stand `id` gelesen haben, können Änderungen verpasst haben: alles
        // außer dem frisch befüllten Verzeichnis neu laden (der Strom spielt ab `id` nach).
        void this.o.qc.invalidateQueries({ predicate: (q) => q.queryKey[0] !== "users" });
      }
    } catch {
      /* Rückfall: ohne gespeicherte ID verbindet connect() live und lädt alles neu */
    }
    if (!this.stopped) this.connect();
  }

  stop(): void {
    this.stopped = true;
    clearTimeout(this.reconnectTimer);
    clearInterval(this.pingTimer);
    const s = this.socket;
    this.socket = null;
    if (s) {
      s.onclose = null;
      s.onmessage = null;
      s.onerror = null;
      s.onopen = null;
      s.close(1000, "client stop");
    }
    this.o.onStatus?.("closed");
  }

  private connect(): void {
    if (this.stopped) return;
    const since = this.o.storage.get();
    this.resumed = since !== null;
    const url = since === null ? this.o.url : `${this.o.url}?since=${since}`;
    this.o.onStatus?.("connecting");
    const socket = (this.o.createSocket ?? ((u) => new WebSocket(u) as unknown as SocketLike))(url);
    this.socket = socket;
    socket.onopen = () => {
      this.lastMessageAt = Date.now();
      this.o.onStatus?.("open");
      clearInterval(this.pingTimer);
      this.pingTimer = setInterval(() => this.ping(), this.o.pingMs ?? 25_000);
    };
    socket.onmessage = (ev) => this.onMessage(socket, ev.data);
    socket.onerror = () => {
      /* onclose folgt immer und plant den Reconnect */
    };
    socket.onclose = (ev) => {
      if (this.socket !== socket) return;
      this.socket = null;
      clearInterval(this.pingTimer);
      this.o.onStatus?.("closed");
      this.scheduleReconnect(ev.code === 4001);
    };
  }

  private ping(): void {
    const dead = this.o.deadAfterMs ?? 70_000;
    if (Date.now() - this.lastMessageAt > dead) {
      this.socket?.close(4000, "dead");
      return;
    }
    try {
      this.socket?.send(JSON.stringify({ type: "ping" }));
    } catch {
      /* onclose folgt */
    }
  }

  private scheduleReconnect(immediate: boolean): void {
    if (this.stopped) return;
    clearTimeout(this.reconnectTimer);
    const { base, max } = this.o.backoffMs ?? { base: 1000, max: 30_000 };
    const exp = Math.min(max, base * 2 ** this.attempts);
    this.attempts += 1;
    const jitter = (this.o.random ?? Math.random)() * 0.3 * exp;
    this.reconnectTimer = setTimeout(() => this.connect(), immediate ? 0 : exp + jitter);
  }

  private persist(id: number): void {
    const cur = this.o.storage.get();
    if (Number.isSafeInteger(id) && (cur === null || id > cur)) this.o.storage.set(id);
  }

  private onMessage(socket: SocketLike, raw: unknown): void {
    if (typeof raw !== "string") return;
    let msg: ServerMessage;
    try {
      msg = JSON.parse(raw) as ServerMessage;
    } catch {
      return;
    }
    this.lastMessageAt = Date.now();
    switch (msg.type) {
      case "hello":
        this.attempts = 0;
        if (!this.resumed || !msg.resumed) {
          // Kein Backlog: der Cache kann hinter dem Server liegen, einmal alles neu laden und
          // ab jetzt vom Kopf des Logs weiterlaufen.
          void this.o.qc.invalidateQueries();
          this.persist(msg.lastSyncId);
        }
        break;
      case "sync":
        if (Array.isArray(msg.actions) && msg.actions.length <= MAX_ACTIONS_PER_MESSAGE) {
          applySyncActions(this.o.qc, msg.actions);
        }
        this.persist(msg.lastSyncId);
        break;
      case "pong":
        this.persist(msg.lastSyncId);
        break;
      case "resetRequired":
        // Lücke größer als die Aufbewahrung: lokalen Stand verwerfen und neu aufsetzen.
        this.o.storage.clear();
        void this.o.qc.invalidateQueries();
        this.socket = null;
        socket.onclose = null;
        socket.close(1000, "reset");
        this.attempts = 0;
        this.scheduleReconnect(true);
        break;
      default:
        break;
    }
  }
}

/** `ws(s)://<host>/sync` passend zur aktuellen Seite. */
export function defaultSyncUrl(loc: Pick<Location, "protocol" | "host"> = window.location): string {
  return `${loc.protocol === "https:" ? "wss" : "ws"}://${loc.host}/sync`;
}
