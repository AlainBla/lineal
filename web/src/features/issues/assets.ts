/**
 * Dateien und Bilder (docs/03 §7, docs/09 §1.7): Upload über `fileUpload` (PUT auf die vorsignierte
 * URL) und Auflösung dauerhafter `assetUrl`s in kurzlebig signierte `/files/…`-URLs. Gespeichert
 * wird nur die `assetUrl` (Markdown `![alt](assetUrl)`); die signierte URL entsteht erst beim
 * Anzeigen und wird nie persistiert. URLs werden auf den eigenen Ursprung reduziert
 * (CSP `img-src 'self'`).
 */
import { gqlRequest } from "@/lib/graphql";

/** Erkennt dauerhafte Asset-URLs (`…/assets/<org>/<uuid>/<uuid>`). */
export function isAssetUrl(src: string | null | undefined): boolean {
  return !!src && /\/assets\/[^/?#]+\/[^/?#]+\/[^/?#]+$/.test(src.split(/[?#]/)[0] ?? "");
}

/** Pfad und Query einer absoluten URL: so bleibt der Zugriff auf dem eigenen Ursprung. */
export function sameOrigin(url: string): string {
  try {
    const u = new URL(url, window.location.origin);
    return `${u.pathname}${u.search}`;
  } catch {
    return url;
  }
}

interface ResolvedAsset {
  assetUrl: string;
  downloadUrl: string;
  expiresAt: string;
}

interface CacheEntry {
  promise: Promise<string | null>;
  /** Spätestens ab hier neu auflösen (ms). */
  until: number;
}

const cache = new Map<string, CacheEntry>();
let pending: Map<string, Array<(url: string | null) => void>> | null = null;
/** Reserve vor Ablauf der signierten URL (ms). */
const REFRESH_MARGIN_MS = 30_000;
const DEFAULT_TTL_MS = 4 * 60_000;

async function flush(batch: Map<string, Array<(url: string | null) => void>>) {
  const urls = [...batch.keys()];
  try {
    const res = await gqlRequest<{ resolveAssets: ResolvedAsset[] }, { urls: string[] }>(
      `query ResolveAssets($urls: [String!]!) { resolveAssets(urls: $urls) { assetUrl downloadUrl expiresAt } }`,
      { urls },
    );
    const found = new Map(res.resolveAssets.map((a) => [a.assetUrl, a]));
    for (const [url, waiters] of batch) {
      const a = found.get(url);
      if (a) {
        const ttl = Date.parse(a.expiresAt) - Date.now() - REFRESH_MARGIN_MS;
        const entry = cache.get(url);
        if (entry) entry.until = Date.now() + (Number.isFinite(ttl) ? ttl : DEFAULT_TTL_MS);
      } else {
        cache.delete(url);
      }
      waiters.forEach((w) => w(a ? sameOrigin(a.downloadUrl) : null));
    }
  } catch {
    for (const [url, waiters] of batch) {
      cache.delete(url);
      waiters.forEach((w) => w(null));
    }
  }
}

/** Liefert die signierte, same-origin Download-URL zu einer `assetUrl` (gebündelt und gecacht). */
export function resolveAssetUrl(assetUrl: string): Promise<string | null> {
  const hit = cache.get(assetUrl);
  if (hit && hit.until > Date.now()) return hit.promise;
  const promise = new Promise<string | null>((resolve) => {
    if (!pending) {
      const batch = new Map<string, Array<(url: string | null) => void>>();
      pending = batch;
      queueMicrotask(() => {
        pending = null;
        void flush(batch);
      });
    }
    pending.set(assetUrl, [...(pending.get(assetUrl) ?? []), resolve]);
  });
  cache.set(assetUrl, { promise, until: Date.now() + DEFAULT_TTL_MS });
  return promise;
}

export function clearAssetCache(): void {
  cache.clear();
}

interface UploadFileResponse {
  fileUpload: {
    uploadFile: {
      uploadUrl: string;
      assetUrl: string;
      headers: Array<{ key: string; value: string }>;
    } | null;
  };
}

/** Lädt eine Datei hoch und liefert die dauerhafte `assetUrl`. */
export async function uploadFile(file: File): Promise<{ assetUrl: string; filename: string }> {
  const contentType = file.type || "application/octet-stream";
  const res = await gqlRequest<
    UploadFileResponse,
    { filename: string; contentType: string; size: number }
  >(
    `mutation FileUpload($filename: String!, $contentType: String!, $size: Int!) {
      fileUpload(filename: $filename, contentType: $contentType, size: $size) {
        success uploadFile { uploadUrl assetUrl headers { key value } }
      }
    }`,
    { filename: file.name, contentType, size: file.size },
  );
  const up = res.fileUpload.uploadFile;
  if (!up) throw new Error("Upload could not be prepared.");
  const headers = new Headers();
  for (const h of up.headers) {
    // Die Länge setzt der Browser selbst.
    if (h.key.toLowerCase() !== "content-length") headers.set(h.key, h.value);
  }
  const put = await fetch(sameOrigin(up.uploadUrl), { method: "PUT", headers, body: file });
  if (!put.ok) throw new Error(`Upload failed (${put.status}).`);
  return { assetUrl: up.assetUrl, filename: file.name };
}

/** Für den Editor: Bild hochladen und die `assetUrl` für das Markdown-Bild liefern. */
export const uploadImageAsset = async (file: File): Promise<string> =>
  (await uploadFile(file)).assetUrl;

export const isImageFile = (f: File) => f.type.startsWith("image/");
