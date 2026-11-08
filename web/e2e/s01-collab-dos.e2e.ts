import { execFileSync } from "node:child_process";
import path from "node:path";

import { expect, test } from "@playwright/test";
import * as encoding from "lib0/encoding";
import * as syncProtocol from "y-protocols/sync";
import * as Y from "yjs";

import { login } from "./helpers";

/**
 * S-01 (LIN-29): Ein Mitbearbeiten-Client schickt über den echten Browser-WebSocket einen tief
 * verschachtelten Baum. Vor dem Fix sprengte das Verwerfen (Drop) eines solchen Baums auf dem
 * Server den Stack und beendete den Prozess. Erwartet: Der Server lehnt die Änderung sauber ab
 * (Schließcode 1009) und bleibt bedienbar.
 *
 * Tiefe 10000 liegt weit über der erlaubten Tiefe (64) und ergibt ~180 KiB, also unter dem
 * 512-KiB-Frame-Limit: so kommt die Änderung bis zur Prüfung/Rücknahme auf dem Server durch.
 * Ein echter Browser kann einen so tiefen Baum nur aufbauen, nicht löschen (auch das JS-yjs
 * sprengt beim Löschen seinen eigenen Stack); der Serverschutz ist das, was zählt.
 */
const CLOSE_TOO_BIG = 1009;

function sqlValue(statement: string): string {
  const cmd = (
    process.env.LINEAL_E2E_PSQL ?? "docker compose exec -T postgres psql -U lineal -d lineal_e2e"
  ).split(/\s+/);
  return execFileSync(
    cmd[0]!,
    [...cmd.slice(1), "-v", "ON_ERROR_STOP=1", "-t", "-A", "-c", statement],
    { cwd: path.resolve(import.meta.dirname, "../.."), stdio: "pipe" },
  )
    .toString()
    .trim();
}

/** Ein y-protocols-Sync-Update-Frame für einen `depth` tief verschachtelten Absatzbaum. */
function deepTreeFrame(depth: number): string {
  const doc = new Y.Doc();
  const frag = doc.getXmlFragment("default");
  doc.transact(() => {
    let el = new Y.XmlElement("paragraph");
    frag.insert(0, [el]);
    for (let i = 1; i < depth; i++) {
      const child = new Y.XmlElement("paragraph");
      el.insert(0, [child]);
      el = child;
    }
  });
  const update = Y.encodeStateAsUpdate(doc);
  const enc = encoding.createEncoder();
  encoding.writeVarUint(enc, 0); // MSG_SYNC
  syncProtocol.writeUpdate(enc, update);
  return Buffer.from(encoding.toUint8Array(enc)).toString("base64");
}

test("S-01: a deep hostile tree is rejected and the server survives", async ({ page }) => {
  const docId = sqlValue("SELECT id FROM document WHERE title = 'Checkout-Spezifikation'");
  expect(docId).toMatch(/^[0-9a-f-]{36}$/);
  const before = sqlValue(
    `SELECT markdown FROM document_content WHERE owner_type = 'document' AND owner_id = '${docId}'`,
  );

  await login(page, "alice");

  const frame = deepTreeFrame(10_000);
  // Grob 180 KiB: groß genug für einen tiefen Baum, klein genug, um das Frame-Limit zu passieren.
  expect(Buffer.from(frame, "base64").byteLength).toBeGreaterThan(100_000);
  expect(Buffer.from(frame, "base64").byteLength).toBeLessThan(512 * 1024);

  // Angriff: im Browser einen WebSocket öffnen (die Session-Cookie reist mit), nach dem
  // Hello-Textframe den feindlichen Baum senden und den Schließcode abwarten.
  const attack = await page.evaluate(
    ({ id, b64 }) => {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const scheme = location.protocol === "https:" ? "wss" : "ws";
      const url = `${scheme}://${location.host}/collab/document/${id}`;
      return new Promise<{ closed: number | null; helloSeen: boolean }>((resolve) => {
        const ws = new WebSocket(url);
        ws.binaryType = "arraybuffer";
        let helloSeen = false;
        const timer = setTimeout(() => {
          try {
            ws.close();
          } catch {
            /* egal */
          }
          resolve({ closed: null, helloSeen });
        }, 20_000);
        ws.onmessage = (ev) => {
          if (typeof ev.data === "string" && !helloSeen) {
            helloSeen = true;
            ws.send(bytes);
          }
        };
        ws.onclose = (ev) => {
          clearTimeout(timer);
          resolve({ closed: ev.code, helloSeen });
        };
      });
    },
    { id: docId, b64: frame },
  );

  expect(attack.helloSeen).toBe(true);
  expect(attack.closed).toBe(CLOSE_TOO_BIG);

  // Der Server lebt noch: Healthcheck, ein frischer Raumbeitritt liefert wieder ein Hello,
  // und der Dokumentinhalt ist unverändert.
  const health = await page.request.get("/healthz");
  expect(health.status()).toBe(200);

  const rejoin = await page.evaluate(
    ({ id }) => {
      const scheme = location.protocol === "https:" ? "wss" : "ws";
      const url = `${scheme}://${location.host}/collab/document/${id}`;
      return new Promise<{ helloSeen: boolean }>((resolve) => {
        const ws = new WebSocket(url);
        ws.binaryType = "arraybuffer";
        const timer = setTimeout(() => {
          try {
            ws.close();
          } catch {
            /* egal */
          }
          resolve({ helloSeen: false });
        }, 15_000);
        ws.onmessage = (ev) => {
          if (typeof ev.data === "string") {
            clearTimeout(timer);
            try {
              ws.close();
            } catch {
              /* egal */
            }
            resolve({ helloSeen: true });
          }
        };
      });
    },
    { id: docId },
  );
  expect(rejoin.helloSeen).toBe(true);

  const after = sqlValue(
    `SELECT markdown FROM document_content WHERE owner_type = 'document' AND owner_id = '${docId}'`,
  );
  expect(after).toBe(before);
});
