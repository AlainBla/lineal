/** Zustand der Bild-Lightbox (LIN-21); öffnet auch aus Node-Views ohne React-Kontext. */
import { useSyncExternalStore } from "react";

export interface LightboxImage {
  /** Anzeigbare (same-origin, ggf. signierte) URL. */
  src: string;
  alt: string;
  /** Dateiname für den Download. */
  filename?: string;
}

/** Offene Galerie: die vorschaubaren Bilder einer Liste und der gezeigte Index (LIN-56). */
export interface LightboxGallery {
  images: LightboxImage[];
  index: number;
}

let gallery: LightboxGallery | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

const clampIndex = (i: number, len: number) => Math.min(Math.max(i, 0), len - 1);

/**
 * Öffnet die Lightbox – mit einem Einzelbild (z. B. ein Inline-Bild) oder mit einer ganzen Liste
 * und dem Startindex, durch die sich dann per Pfeil blättern lässt (LIN-56). Eine leere Liste
 * öffnet nichts.
 */
export function openLightbox(image: LightboxImage): void;
export function openLightbox(images: LightboxImage[], index: number): void;
export function openLightbox(arg: LightboxImage | LightboxImage[], index = 0): void {
  const images = Array.isArray(arg) ? arg : [arg];
  if (images.length === 0) return;
  gallery = { images, index: clampIndex(index, images.length) };
  emit();
}

/** Blättert um `step` Bilder weiter; am Anfang/Ende ein No-op (kein Umlauf). */
function step(delta: number): void {
  if (!gallery) return;
  const next = clampIndex(gallery.index + delta, gallery.images.length);
  if (next !== gallery.index) {
    gallery = { images: gallery.images, index: next };
    emit();
  }
}

export function nextLightbox(): void {
  step(1);
}

export function prevLightbox(): void {
  step(-1);
}

export function closeLightbox(): void {
  gallery = null;
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

/** Endung zu einem erlaubten Bildtyp, sonst `undefined`. */
export function imageExtension(type: string): string | undefined {
  return EXT[type.split(";")[0]!.trim().toLowerCase()];
}

/**
 * Dateiname für einen Bild-Download: die Endung kommt immer aus dem tatsächlichen Inhaltstyp
 * (ein Alt-Text wie „rechnung.exe“ wird zu „rechnung.png“); Steuer- und Bidi-Zeichen fliegen raus.
 */
export function downloadName(name: string | undefined, ext: string): string {
  const clean = (name ?? "")
    .replace(/\p{C}/gu, "")
    .replace(/[\\/:*?"<>|]+/g, "_")
    .trim();
  const base = clean.replace(/\.[^.]*$/, "").trim() || "image";
  return `${base}.${ext}`;
}

/** Nur http(s)-URLs des eigenen Ursprungs (Assets, signierte `/files/…`). */
function sameOriginHref(url: string): string | null {
  try {
    const u = new URL(url, window.location.origin);
    if (u.origin !== window.location.origin) return null;
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}

/**
 * Lädt ein Bild des eigenen Ursprungs herunter. Über einen Blob, weil der Server per
 * `Content-Disposition` sonst den Speicherschlüssel als Namen vorgibt. Ist der Inhalt kein
 * erlaubter Bildtyp oder schlägt das Laden fehl, gilt der direkte Link mit dem Namen des Servers.
 * Fremde oder nicht-http(s)-URLs werden ignoriert.
 */
export async function downloadUrl(url: string, filename?: string): Promise<void> {
  const href = sameOriginHref(url);
  if (!href) return;
  try {
    const res = await fetch(href);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const blob = await res.blob();
    const ext = imageExtension(blob.type);
    if (!ext) throw new Error("not an image");
    const objectUrl = URL.createObjectURL(blob);
    clickLink(objectUrl, downloadName(filename, ext));
    setTimeout(() => URL.revokeObjectURL(objectUrl), 10_000);
  } catch {
    clickLink(href, "");
  }
}

export function useLightboxGallery(): LightboxGallery | null {
  return useSyncExternalStore(
    subscribe,
    () => gallery,
    () => null,
  );
}
