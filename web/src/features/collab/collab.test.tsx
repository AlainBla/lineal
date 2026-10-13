import { render, screen } from "@testing-library/react";
import * as decoding from "lib0/decoding";
import * as encoding from "lib0/encoding";
import * as awarenessProtocol from "y-protocols/awareness";
import * as syncProtocol from "y-protocols/sync";
import * as Y from "yjs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Presence, participantsOf } from "./presence";
import { CollabProvider, collabUrl } from "./provider";

class FakeSocket {
  static instances: FakeSocket[] = [];
  readyState = 0;
  binaryType = "blob";
  sent: (string | Uint8Array)[] = [];
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.instances.push(this);
  }
  send(data: string | Uint8Array) {
    this.sent.push(data);
  }
  close() {
    this.readyState = 3;
  }
  open() {
    this.readyState = 1;
  }
  text(obj: unknown) {
    this.onmessage?.({ data: JSON.stringify(obj) });
  }
  binary(bytes: Uint8Array) {
    const copy = bytes.slice().buffer;
    this.onmessage?.({ data: copy });
  }
  drop(code = 1006) {
    this.readyState = 3;
    this.onclose?.({ code });
  }
  binaries(): Uint8Array[] {
    return this.sent.filter((s): s is Uint8Array => typeof s !== "string");
  }
}

const Impl = FakeSocket as unknown as typeof WebSocket;
const hello = (over: Record<string, unknown> = {}) => ({
  type: "hello",
  epoch: "e1",
  canWrite: true,
  version: 1,
  user: { name: "Alice Example", color: "#0091ff" },
  ...over,
});

function make(opts: Partial<ConstructorParameters<typeof CollabProvider>[0]> = {}) {
  const p = new CollabProvider({
    url: "ws://x/collab/document/d1",
    WebSocketImpl: Impl,
    random: () => 1,
    ...opts,
  });
  p.connect();
  return p;
}

/** Server-Sync-Schritt 1 (leerer Zustandsvektor), wie ihn der Server nach dem Hello sendet. */
function serverStep1(): Uint8Array {
  const enc = encoding.createEncoder();
  encoding.writeVarUint(enc, 0);
  syncProtocol.writeSyncStep1(enc, new Y.Doc());
  return encoding.toUint8Array(enc);
}

function syncOf(frame: Uint8Array): { kind: number; payload: Uint8Array } | null {
  const d = decoding.createDecoder(frame);
  if (decoding.readVarUint(d) !== 0) return null;
  return { kind: decoding.readVarUint(d), payload: decoding.readVarUint8Array(d) };
}

beforeEach(() => {
  FakeSocket.instances = [];
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("CollabProvider", () => {
  it("baut die URL passend zur Seite", () => {
    expect(
      collabUrl("document", "a b", { protocol: "https:", host: "lineal.example" } as Location),
    ).toBe("wss://lineal.example/collab/document/a%20b");
    expect(
      collabUrl("issue", "d1", { protocol: "http:", host: "localhost:5173" } as Location),
    ).toBe("ws://localhost:5173/collab/issue/d1");
  });

  it("verbindet, sendet Schritt 1 und wendet Serverstand an", () => {
    const p = make();
    const ws = FakeSocket.instances[0]!;
    ws.open();
    expect(p.status).toBe("connecting");
    ws.text(hello());
    expect(p.status).toBe("connected");
    expect(p.canWrite).toBe(true);
    expect(p.identity?.name).toBe("Alice Example");
    const first = ws
      .binaries()
      .map(syncOf)
      .find((m) => m)!;
    expect(first.kind).toBe(syncProtocol.messageYjsSyncStep1);

    // Serverstand (Schritt 2) kommt an.
    const server = new Y.Doc();
    server.getText("t").insert(0, "Hallo");
    const enc = encoding.createEncoder();
    encoding.writeVarUint(enc, 0);
    syncProtocol.writeSyncStep2(enc, server);
    ws.binary(encoding.toUint8Array(enc));
    expect(p.doc.getText("t").toString()).toBe("Hallo");
    expect(p.synced).toBe(true);
    p.destroy();
  });

  it("sendet lokale Änderungen nur mit Schreibrecht", () => {
    const rw = make();
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    const before = a.binaries().length;
    rw.doc.getText("t").insert(0, "x");
    expect(a.binaries().length).toBe(before + 1);
    expect(syncOf(a.binaries().at(-1)!)!.kind).toBe(syncProtocol.messageYjsUpdate);
    rw.destroy();

    const ro = make();
    const b = FakeSocket.instances[1]!;
    b.open();
    b.text(hello({ canWrite: false }));
    expect(ro.canWrite).toBe(false);
    const sent = b.binaries().length;
    ro.doc.getText("t").insert(0, "nicht senden");
    // Auch die Antwort auf Schritt 1 des Servers enthält für Nur-Leser nichts.
    b.binary(serverStep1());
    expect(b.binaries().length).toBe(sent);
    ro.destroy();
  });

  it("verbindet mit Backoff neu und reicht Offline-Änderungen nach", () => {
    const p = make({ baseDelayMs: 100, maxDelayMs: 1000 });
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    a.drop(1006);
    expect(p.status).toBe("disconnected");
    // Offline getippt.
    p.doc.getText("t").insert(0, "offline");
    expect(FakeSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(100);
    expect(FakeSocket.instances).toHaveLength(2);
    const b = FakeSocket.instances[1]!;
    b.open();
    b.text(hello());
    expect(p.status).toBe("connected");
    // Der Server fragt mit leerem Zustandsvektor an: die Offline-Änderung geht als Schritt 2 raus.
    b.binary(serverStep1());
    const reply = b
      .binaries()
      .map(syncOf)
      .find((m) => m?.kind === syncProtocol.messageYjsSyncStep2)!;
    const check = new Y.Doc();
    Y.applyUpdate(check, reply.payload);
    expect(check.getText("t").toString()).toBe("offline");
    // Jeder weitere Fehlschlag verdoppelt die Wartezeit (auch nach einem Hello ohne stabile Phase).
    b.drop(1006);
    vi.advanceTimersByTime(199);
    expect(FakeSocket.instances).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(3);
    FakeSocket.instances[2]!.drop(1006);
    vi.advanceTimersByTime(399);
    expect(FakeSocket.instances).toHaveLength(3);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(4);
    p.destroy();
  });

  it("Backoff wächst bei Ablehnung nach dem Handshake (4400), Reset erst nach stabiler Phase", () => {
    const p = make({ baseDelayMs: 100, maxDelayMs: 10_000, stableMs: 5000 });
    const delays: number[] = [];
    let n = 1;
    for (let i = 0; i < 4; i++) {
      const ws = FakeSocket.instances[n - 1]!;
      ws.open();
      ws.text(hello());
      ws.drop(4400);
      let waited = 0;
      while (FakeSocket.instances.length === n && waited < 20_000) {
        vi.advanceTimersByTime(1);
        waited += 1;
      }
      delays.push(waited);
      n += 1;
    }
    expect(delays).toEqual([100, 200, 400, 800]);
    // Stabile Verbindung setzt den Backoff zurück.
    const ok = FakeSocket.instances[n - 1]!;
    ok.open();
    ok.text(hello());
    vi.advanceTimersByTime(5000);
    ok.drop(1006);
    vi.advanceTimersByTime(100);
    expect(FakeSocket.instances).toHaveLength(n + 1);
    p.destroy();
  });

  it("lehnt zu große Änderungen ehrlich ab, statt Offline zu melden und endlos zu verbinden", () => {
    const p = make({ baseDelayMs: 10, maxUpdateBytes: 1000 });
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    const before = a.binaries().length;
    p.doc.getText("t").insert(0, "x".repeat(5000));
    expect(p.status).toBe("rejected");
    expect(p.canWrite).toBe(false);
    expect(p.closeCode).toBe(1009);
    expect(a.binaries().length).toBe(before);
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(1);
    p.destroy();
  });

  it("verbindet nach Serverschluss 1009 nicht erneut", () => {
    const p = make({ baseDelayMs: 10 });
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    a.drop(1009);
    expect(p.status).toBe("rejected");
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(1);
    p.destroy();
  });

  it("fällt auf Stufe 1 zurück, wenn der Endpunkt nie erreichbar ist", () => {
    const p = make({ baseDelayMs: 10, maxInitialFailures: 3 });
    FakeSocket.instances[0]!.drop(1006);
    vi.advanceTimersByTime(10);
    FakeSocket.instances[1]!.drop(1006);
    vi.advanceTimersByTime(20);
    FakeSocket.instances[2]!.drop(1006);
    expect(p.status).toBe("fallback");
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(3);
    p.destroy();
  });

  it("beendet bei 4401 ohne Wiederverbindung", () => {
    const p = make();
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    a.drop(4401);
    expect(p.status).toBe("closed");
    expect(p.closeCode).toBe(4401);
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(1);
    p.destroy();
  });

  it("meldet einen Epochenwechsel, statt zusammenzuführen", () => {
    const p = make({ baseDelayMs: 10 });
    const reset = vi.fn();
    p.onReset(reset);
    const a = FakeSocket.instances[0]!;
    a.open();
    a.text(hello());
    a.drop(1006);
    vi.advanceTimersByTime(10);
    const b = FakeSocket.instances[1]!;
    b.open();
    b.text(hello({ epoch: "e2" }));
    expect(reset).toHaveBeenCalledTimes(1);
    p.destroy();
  });
});

describe("Teilnehmeranzeige", () => {
  function awarenessFrom(
    states: Record<number, unknown>,
    clock = 1,
  ): { frame: Uint8Array; ids: number[] } {
    const enc = encoding.createEncoder();
    encoding.writeVarUint(enc, 1);
    const update = encoding.createEncoder();
    const ids = Object.keys(states).map(Number);
    encoding.writeVarUint(update, ids.length);
    for (const id of ids) {
      encoding.writeVarUint(update, id);
      encoding.writeVarUint(update, clock);
      encoding.writeVarString(update, JSON.stringify(states[id]));
    }
    encoding.writeVarUint8Array(enc, encoding.toUint8Array(update));
    return { frame: encoding.toUint8Array(enc), ids };
  }

  it("zeigt Namen und Cursorfarben der Teilnehmer", () => {
    vi.useRealTimers();
    const p = make();
    const ws = FakeSocket.instances[0]!;
    ws.open();
    ws.text(hello());
    ws.binary(
      awarenessFrom({
        4242: { user: { name: "Bob Beispiel", color: "#e5484d" } },
        4243: { user: { name: "x", color: "javascript:alert(1)" } },
        4244: { cursor: {} },
      }).frame,
    );
    const people = participantsOf(p);
    const names = people.map((x) => x.name);
    expect(names).toContain("Alice Example");
    expect(names).toContain("Bob Beispiel");
    expect(people.find((x) => x.name === "Bob Beispiel")?.color).toBe("#e5484d");
    // Unsichere Farbwerte werden nicht übernommen.
    expect(people.find((x) => x.name === "x")?.color).toBe("#888888");
    expect(people.some((x) => x.self && x.name === "Alice Example")).toBe(true);

    render(<Presence provider={p} />);
    expect(screen.getByLabelText("Bob Beispiel")).toHaveTextContent("BB");
    expect(screen.getByLabelText("Alice Example (you)")).toBeInTheDocument();
    p.destroy();
  });

  it("sendet den eigenen Cursorzustand als Awareness", () => {
    const p = make();
    const ws = FakeSocket.instances[0]!;
    ws.open();
    ws.text(hello());
    const types = ws.binaries().map((b) => decoding.readVarUint(decoding.createDecoder(b)));
    expect(types).toContain(1);
    const own = awarenessProtocol.encodeAwarenessUpdate(p.awareness, [p.doc.clientID]);
    expect(own.length).toBeGreaterThan(0);
    p.destroy();
  });
});
