/**
 * Client für `GET /sync/bootstrap` (docs/09 §5.2): lädt den Bestand in Paketen als zeilenweises JSON.
 *
 * Jede Zeile ist ein Objekt mit `__class` (Modellname); die letzte Zeile jedes Pakets ist
 * `_metadata_={"lastSyncId":N,"nextCursor":…}`. `lastSyncId` ist der Stand des Logs **vor** dem Lesen:
 * Wer danach `/sync?since=<lastSyncId>` öffnet, erhält alles, was währenddessen passiert ist.
 */

export interface BootstrapObject {
  __class: string;
  id: string;
  [field: string]: unknown;
}

export interface BootstrapResult {
  /** Stand des Sync-Logs beim ersten Paket; Startpunkt für `?since=`. */
  lastSyncId: number;
  objects: BootstrapObject[];
}

interface Meta {
  lastSyncId: number;
  nextCursor: string | null;
  databaseVersion?: number;
}

const META_PREFIX = "_metadata_=";
/** Schutz vor Endlosschleifen bei fehlerhaften Servern. */
const MAX_PAGES = 50;

/** Zerlegt eine Antwort in Objekte und Metadaten; `null`, wenn die Metazeile fehlt oder ungültig ist. */
export function parseBootstrapPage(
  text: string,
): { objects: BootstrapObject[]; meta: Meta } | null {
  const objects: BootstrapObject[] = [];
  let meta: Meta | null = null;
  for (const line of text.split("\n")) {
    if (!line) continue;
    if (line.startsWith(META_PREFIX)) {
      try {
        const m = JSON.parse(line.slice(META_PREFIX.length)) as Meta;
        if (Number.isSafeInteger(m.lastSyncId) && m.lastSyncId >= 0) meta = m;
      } catch {
        return null;
      }
      continue;
    }
    try {
      const o = JSON.parse(line) as BootstrapObject;
      if (o && typeof o === "object" && typeof o.__class === "string") objects.push(o);
    } catch {
      return null;
    }
  }
  return meta ? { objects, meta } : null;
}

export interface FetchBootstrapOptions {
  fetchImpl?: typeof fetch;
  /** Basis ohne Query, Default `/sync/bootstrap`. */
  url?: string;
  limit?: number;
  signal?: AbortSignal;
}

/** Lädt alle Pakete der Modelle. Wirft bei HTTP- oder Formatfehlern. */
export async function fetchBootstrap(
  models: readonly string[],
  opts: FetchBootstrapOptions = {},
): Promise<BootstrapResult> {
  const doFetch = opts.fetchImpl ?? fetch;
  const base = opts.url ?? "/sync/bootstrap";
  const objects: BootstrapObject[] = [];
  let cursor: string | null = null;
  let lastSyncId: number | null = null;
  for (let page = 0; page < MAX_PAGES; page++) {
    const params = new URLSearchParams();
    if (cursor) params.set("cursor", cursor);
    else params.set("modelNames", models.join(","));
    if (opts.limit) params.set("limit", String(opts.limit));
    const res = await doFetch(`${base}?${params.toString()}`, {
      credentials: "same-origin",
      headers: { Accept: "application/x-ndjson" },
      signal: opts.signal,
    });
    if (!res.ok) throw new Error(`bootstrap failed: HTTP ${res.status}`);
    const parsed = parseBootstrapPage(await res.text());
    if (!parsed) throw new Error("bootstrap failed: malformed response");
    objects.push(...parsed.objects);
    // Der Stand des ersten Pakets gilt für alle Folgepakete (steckt im Cursor).
    lastSyncId ??= parsed.meta.lastSyncId;
    if (!parsed.meta.nextCursor) return { lastSyncId, objects };
    cursor = parsed.meta.nextCursor;
  }
  throw new Error("bootstrap failed: too many pages");
}
