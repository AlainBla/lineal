/**
 * Emoji-Kurzcodes wie in Linear/GitHub (`:bar_chart:` → 📊). Die Daten (gemoji, ~1900 Einträge)
 * werden erst bei Bedarf nachgeladen; bis dahin bleiben Kurzcodes als Text stehen.
 */
import { useEffect, useState } from "react";

export interface EmojiEntry {
  emoji: string;
  /** Kurzcodes ohne Doppelpunkte (`bar_chart`). */
  names: string[];
  tags: string[];
}

export interface EmojiData {
  byName: ReadonlyMap<string, string>;
  list: readonly EmojiEntry[];
}

let loaded: EmojiData | null = null;
let loading: Promise<EmojiData> | null = null;

export function loadEmoji(): Promise<EmojiData> {
  if (loaded) return Promise.resolve(loaded);
  loading ??= import("gemoji").then(({ gemoji }) => {
    const byName = new Map<string, string>();
    for (const e of gemoji) for (const n of e.names) byName.set(n, e.emoji);
    loaded = { byName, list: gemoji };
    return loaded;
  });
  return loading;
}

/** Bereits geladene Daten (ohne Laden anzustoßen). */
export function emojiDataSync(): EmojiData | null {
  return loaded;
}

/** Lädt die Emoji-Daten und rendert neu, sobald sie da sind. */
export function useEmojiData(): EmojiData | null {
  const [data, setData] = useState<EmojiData | null>(loaded);
  useEffect(() => {
    if (data) return;
    let alive = true;
    void loadEmoji().then((d) => alive && setData(d));
    return () => {
      alive = false;
    };
  }, [data]);
  return data;
}

const SHORTCODE = /^:([a-z0-9_+-]+):$/i;

/**
 * Anzeigetext eines Symbols (Dokument, Team, Projekt …): Kurzcode → Emoji, Emoji bleibt.
 * `null` für leere Werte und Linear-Symbolnamen ohne Emoji-Entsprechung (z. B. `Rocket`).
 */
export function iconEmoji(icon: string | null | undefined, data: EmojiData | null): string | null {
  const v = icon?.trim();
  if (!v) return null;
  const m = SHORTCODE.exec(v);
  if (m) return data?.byName.get(m[1]!.toLowerCase()) ?? (data ? null : v);
  // Ein Emoji (auch mit Varianten/ZWJ) besteht nicht aus ASCII-Buchstaben.
  return /^[\p{L}\p{N}\s_-]+$/u.test(v) ? null : v;
}

/** Vorschläge zu einer Eingabe nach `:` (Präfix vor Teilwort, höchstens `limit`). */
export function searchEmoji(data: EmojiData, query: string, limit = 8): EmojiEntry[] {
  const q = query.toLowerCase();
  if (!q) return [];
  const prefix: EmojiEntry[] = [];
  const partial: EmojiEntry[] = [];
  for (const e of data.list) {
    if (e.names.some((n) => n.startsWith(q))) prefix.push(e);
    else if (e.names.some((n) => n.includes(q)) || e.tags.some((t) => t.startsWith(q)))
      partial.push(e);
    if (prefix.length >= limit) break;
  }
  return [...prefix, ...partial].slice(0, limit);
}
