/**
 * Zustand der Markdown-Vorschau (LIN-43): zeigt einen `.md`-Anhang gerendert in Lineal an, statt
 * einen Download auszulösen. Zustand und `openMarkdownPreview` liegen hier, die Ansicht hängt
 * einmal im `IssuesHost` (`MarkdownPreview`).
 */
import { useSyncExternalStore } from "react";

export interface MarkdownPreview {
  /** Anzeigbare (same-origin, ggf. signierte) URL der Markdown-Datei. */
  src: string;
  /** Titel/Dateiname für Überschrift und Download. */
  title: string;
}

let current: MarkdownPreview | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function openMarkdownPreview(preview: MarkdownPreview): void {
  current = preview;
  emit();
}

export function closeMarkdownPreview(): void {
  current = null;
  emit();
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useMarkdownPreview(): MarkdownPreview | null {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current,
  );
}

/** Erkennt einen Markdown-Anhang an Inhaltstyp oder Dateiendung. */
export function isMarkdown(contentType: string | null | undefined, title: string): boolean {
  const essence = (contentType ?? "").split(";")[0]!.trim().toLowerCase();
  return essence === "text/markdown" || essence === "text/x-markdown" || /\.(md|markdown)$/i.test(title);
}
