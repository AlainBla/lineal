/**
 * Schnellfilter im Suchfeld der Topbar: filtert die sichtbaren Issues der aktuellen Liste/Board live
 * im Client. Globaler Store (wie `ui-state.ts`); Listenseiten melden ihre Trefferzahl über
 * `useQuickFiltered` / `useQuickFilterCount` an, die Topbar zeigt sie an und weiß so, ob die Seite
 * überhaupt eine Issue-Liste hat.
 */
import { useEffect, useMemo, useSyncExternalStore } from "react";

interface QuickFilterState {
  text: string;
  /** Trefferzahl der aktuellen Listenseite; `null`, wenn die Seite keine Issue-Liste zeigt. */
  count: number | null;
}

let state: QuickFilterState = { text: "", count: null };
const listeners = new Set<() => void>();

function set(patch: Partial<QuickFilterState>) {
  const next = { ...state, ...patch };
  if (next.text === state.text && next.count === state.count) return;
  state = next;
  listeners.forEach((l) => l());
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const getState = () => state;

export const getQuickFilterState = (): QuickFilterState => state;
export const setQuickFilter = (text: string) => set({ text });
export const clearQuickFilter = () => set({ text: "" });
export const useQuickFilterState = (): QuickFilterState =>
  useSyncExternalStore(subscribe, getState, getState);
export const useQuickFilter = (): string => useQuickFilterState().text;

/**
 * Passt ein Issue auf den Schnellfilter? Kennung: Präfix (`KOL-16` → KOL-16, KOL-160, …; eine reine
 * Zahl passt auf den Zahlenteil), Titel: Teilstring; beides ohne Beachtung der Groß-/Kleinschreibung.
 */
export function matchesQuickFilter(
  issue: { identifier: string; title: string },
  text: string,
): boolean {
  const q = text.trim().toLowerCase();
  if (!q) return true;
  const id = issue.identifier.toLowerCase();
  if (id.startsWith(q)) return true;
  if (/^\d+$/.test(q) && id.slice(id.lastIndexOf("-") + 1).startsWith(q)) return true;
  return issue.title.toLowerCase().includes(q);
}

export function applyQuickFilter<T extends { identifier: string; title: string }>(
  issues: T[],
  text: string,
): T[] {
  return text.trim() ? issues.filter((i) => matchesQuickFilter(i, text)) : issues;
}

/** Meldet die Trefferzahl der Seite an die Topbar; beim Verlassen der Seite wieder ab. */
export function useQuickFilterCount(count: number) {
  useEffect(() => () => set({ count: null }), []);
  useEffect(() => set({ count }), [count]);
}

/** Filtert die sichtbaren Issues einer Liste mit dem Schnellfilter und meldet die Trefferzahl. */
export function useQuickFiltered<T extends { identifier: string; title: string }>(
  issues: T[],
): T[] {
  const text = useQuickFilter();
  const out = useMemo(() => applyQuickFilter(issues, text), [issues, text]);
  useQuickFilterCount(out.length);
  return out;
}
