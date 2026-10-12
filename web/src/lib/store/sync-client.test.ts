import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { localStorageSync, SyncClient, type SocketLike, type SyncStorage } from "./sync-client";

class FakeSocket implements SocketLike {
  onopen: SocketLike["onopen"] = null;
  onmessage: SocketLike["onmessage"] = null;
  onclose: SocketLike["onclose"] = null;
  onerror: SocketLike["onerror"] = null;
  sent: string[] = [];
  closed = false;
  constructor(readonly url: string) {}
  send(d: string) {
    this.sent.push(d);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.onopen?.({});
  }
  push(msg: unknown) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  drop(code = 1006) {
    this.onclose?.({ code });
  }
}

function memoryStorage(initial: number | null = null): SyncStorage & { value: number | null } {
  const s = {
    value: initial,
    get: () => s.value,
    set: (id: number) => {
      s.value = id;
    },
    clear: () => {
      s.value = null;
    },
  };
  return s;
}

describe("SyncClient", () => {
  let sockets: FakeSocket[];
  let qc: QueryClient;

  const make = (storage: SyncStorage) =>
    new SyncClient({
      qc,
      storage,
      url: "ws://x/sync",
      createSocket: (u) => {
        const s = new FakeSocket(u);
        sockets.push(s);
        return s;
      },
      backoffMs: { base: 1000, max: 4000 },
      random: () => 0,
    });

  beforeEach(() => {
    vi.useFakeTimers();
    sockets = [];
    qc = new QueryClient();
  });
  afterEach(() => vi.useRealTimers());

  it("Bootstrap beim Erststart: Strom ab dessen lastSyncId, ohne Voll-Reload des Verzeichnisses", async () => {
    const storage = memoryStorage();
    const spy = vi.spyOn(qc, "invalidateQueries");
    qc.setQueryData(["users", "u", "ws", "", ""], [{ id: "1" }]);
    const bootstrap = vi.fn(async () => 77);
    const client = new SyncClient({
      qc,
      storage,
      url: "ws://x/sync",
      bootstrap,
      createSocket: (u) => {
        const s = new FakeSocket(u);
        sockets.push(s);
        return s;
      },
    });
    client.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(bootstrap).toHaveBeenCalledTimes(1);
    expect(storage.value).toBe(77);
    expect(sockets[0]!.url).toBe("ws://x/sync?since=77");
    // Alles außer dem befüllten Verzeichnis wird neu geladen.
    const call = spy.mock.calls.find((c) => c[0] && "predicate" in c[0])!;
    const predicate = (call[0] as unknown as { predicate: (q: { queryKey: unknown[] }) => boolean })
      .predicate;
    expect(predicate({ queryKey: ["users", "u"] })).toBe(false);
    expect(predicate({ queryKey: ["issues"] })).toBe(true);
    // hello (resumed) löst keinen weiteren Voll-Reload aus.
    spy.mockClear();
    sockets[0]!.open();
    sockets[0]!.push({ type: "hello", lastSyncId: 80, resumed: true });
    expect(spy).not.toHaveBeenCalled();
    client.stop();
  });

  it("Bootstrap schlägt fehl: Rückfall auf den bisherigen Ablauf", async () => {
    const storage = memoryStorage();
    const client = new SyncClient({
      qc,
      storage,
      url: "ws://x/sync",
      bootstrap: async () => {
        throw new Error("offline");
      },
      createSocket: (u) => {
        const s = new FakeSocket(u);
        sockets.push(s);
        return s;
      },
    });
    client.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(storage.value).toBeNull();
    expect(sockets[0]!.url).toBe("ws://x/sync");
    client.stop();
  });

  it("mit gespeicherter ID kein Bootstrap", () => {
    const bootstrap = vi.fn(async () => 1);
    const client = new SyncClient({
      qc,
      storage: memoryStorage(5),
      url: "ws://x/sync",
      bootstrap,
      createSocket: (u) => {
        const s = new FakeSocket(u);
        sockets.push(s);
        return s;
      },
    });
    client.start();
    expect(bootstrap).not.toHaveBeenCalled();
    expect(sockets[0]!.url).toBe("ws://x/sync?since=5");
    client.stop();
  });

  it("verbindet ohne since, lädt einmal alles neu und merkt sich den Kopf des Logs", () => {
    const storage = memoryStorage();
    const spy = vi.spyOn(qc, "invalidateQueries");
    const client = make(storage);
    client.start();
    expect(sockets[0]!.url).toBe("ws://x/sync");
    sockets[0]!.open();
    sockets[0]!.push({ type: "hello", lastSyncId: 42, resumed: false });
    expect(storage.value).toBe(42);
    expect(spy).toHaveBeenCalledWith();
    client.stop();
  });

  it("wendet Pakete an, speichert die ID und holt nach Verbindungsverlust die Lücke mit since", () => {
    const storage = memoryStorage(42);
    const client = make(storage);
    client.start();
    expect(sockets[0]!.url).toBe("ws://x/sync?since=42");
    sockets[0]!.open();
    // hello einer fortgesetzten Sitzung überschreibt die gespeicherte ID nicht (Backlog folgt)
    sockets[0]!.push({ type: "hello", lastSyncId: 99, resumed: true });
    expect(storage.value).toBe(42);
    const spy = vi.spyOn(qc, "invalidateQueries");
    sockets[0]!.push({
      type: "sync",
      lastSyncId: 50,
      actions: [{ id: 50, action: "I", modelName: "Notification", modelId: "n", data: {} }],
    });
    expect(storage.value).toBe(50);
    expect(spy).toHaveBeenCalledWith({ queryKey: ["sidebar-data"] });

    sockets[0]!.drop();
    expect(sockets).toHaveLength(1); // Backoff läuft
    vi.advanceTimersByTime(1000);
    expect(sockets).toHaveLength(2);
    expect(sockets[1]!.url).toBe("ws://x/sync?since=50");
    client.stop();
  });

  it("Backoff verdoppelt sich bis zur Obergrenze und startet nach hello neu", () => {
    const client = make(memoryStorage(1));
    client.start();
    for (const wait of [1000, 2000, 4000, 4000]) {
      sockets.at(-1)!.drop();
      vi.advanceTimersByTime(wait - 1);
      const before = sockets.length;
      vi.advanceTimersByTime(1);
      expect(sockets.length).toBe(before + 1);
    }
    sockets.at(-1)!.open();
    sockets.at(-1)!.push({ type: "hello", lastSyncId: 1, resumed: true });
    sockets.at(-1)!.drop();
    vi.advanceTimersByTime(1000);
    expect(sockets.length).toBe(6);
    client.stop();
  });

  it("resetRequired verwirft die gespeicherte ID, lädt neu und verbindet ohne since", () => {
    const storage = memoryStorage(7);
    const spy = vi.spyOn(qc, "invalidateQueries");
    const client = make(storage);
    client.start();
    sockets[0]!.open();
    sockets[0]!.push({ type: "resetRequired" });
    expect(storage.value).toBeNull();
    expect(spy).toHaveBeenCalledWith();
    expect(sockets[0]!.closed).toBe(true);
    vi.advanceTimersByTime(10);
    expect(sockets[1]!.url).toBe("ws://x/sync");
    client.stop();
  });

  it("sendet App-Pings und trennt tote Verbindungen", () => {
    const client = new SyncClient({
      qc,
      storage: memoryStorage(),
      url: "ws://x/sync",
      pingMs: 1000,
      deadAfterMs: 2500,
      createSocket: (u) => {
        const s = new FakeSocket(u);
        sockets.push(s);
        return s;
      },
    });
    client.start();
    sockets[0]!.open();
    vi.advanceTimersByTime(1000);
    expect(sockets[0]!.sent).toEqual([JSON.stringify({ type: "ping" })]);
    sockets[0]!.push({ type: "pong", lastSyncId: 3 });
    vi.advanceTimersByTime(3000);
    expect(sockets[0]!.closed).toBe(true);
    client.stop();
  });

  it("stop beendet alles, ohne erneut zu verbinden", () => {
    const client = make(memoryStorage());
    client.start();
    client.stop();
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(1);
  });
});

describe("localStorageSync", () => {
  it("speichert je Scope und ignoriert ungültige Werte", () => {
    const store = new Map<string, string>();
    const fake = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    } as unknown as Storage;
    const a = localStorageSync("acme:u1", fake);
    const b = localStorageSync("acme:u2", fake);
    expect(a.get()).toBeNull();
    a.set(12);
    expect(a.get()).toBe(12);
    expect(b.get()).toBeNull();
    store.set("lineal.sync.lastSyncId.acme:u2", "abc");
    expect(b.get()).toBeNull();
    a.clear();
    expect(a.get()).toBeNull();
  });
});
