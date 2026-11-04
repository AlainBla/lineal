/** Zustand der Bild-Lightbox (LIN-21); öffnet auch aus Node-Views ohne React-Kontext. */
import { useSyncExternalStore } from "react";

export interface LightboxImage {
  /** Anzeigbare (same-origin, ggf. signierte) URL. */
  src: string;
  alt: string;
  /** Dateiname für den Download. */
  filename?: string;
}

let current: LightboxImage | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function openLightbox(image: LightboxImage): void {
  current = image;
  emit();
}

export function closeLightbox(): void {
  current = null;
  emit();
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

function clickLink(href: string, filename: string): void {
  const a = document.createElement("a");
  a.href = href;
  a.download = filename;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
}

const EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/svg+xml": "svg",
  "image/avif": "avif",
};

/** Dateiname mit Endung: Alt-Texte von Bildern tragen oft keine. */
export function downloadName(name: string | undefined, type: string): string {
  const base = (name ?? "").trim().replace(/[\\/:*?"<>|]+/g, "_") || "image";
  const ext = EXT[type.split(";")[0]!.trim().toLowerCase()];
  return ext && !/\.[a-z0-9]{2,5}$/i.test(base) ? `${base}.${ext}` : base;
}

/**
 * Lädt eine (same-origin) URL unter dem gegebenen Dateinamen herunter. Über einen Blob, weil der
 * Server per `Content-Disposition` sonst den Speicherschlüssel als Namen vorgibt; schlägt das
 * Laden fehl, bleibt der direkte Link.
 */
export async function downloadUrl(url: string, filename?: string): Promise<void> {
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const blob = await res.blob();
    const href = URL.createObjectURL(blob);
    clickLink(href, downloadName(filename, blob.type));
    setTimeout(() => URL.revokeObjectURL(href), 10_000);
  } catch {
    clickLink(url, filename ?? "");
  }
}

export function useLightboxImage(): LightboxImage | null {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => null,
  );
}
