/**
 * Mehrfachauswahl in Liste und Board (docs/08 §2.1): Shift-Klick wählt einen Bereich, Cmd/Strg-Klick
 * und `x` schalten einzelne Issues um, Escape leert. Die Auswahl besteht nur aus Issues, die in der
 * aktuellen Ansicht noch vorkommen (gefilterte oder verschobene fallen automatisch heraus).
 */
import { useCallback, useMemo, useRef, useState } from "react";

/** Bereich zwischen Anker und Ziel (beide inklusive) in der Anzeigereihenfolge. */
export function rangeBetween(ordered: string[], anchor: string | null, to: string): string[] {
  const end = ordered.indexOf(to);
  if (end < 0) return [];
  const start = anchor === null ? -1 : ordered.indexOf(anchor);
  if (start < 0) return [to];
  const [a, b] = start <= end ? [start, end] : [end, start];
  return ordered.slice(a, b + 1);
}

export function toggled(set: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(set);
  if (!next.delete(id)) next.add(id);
  return next;
}

export interface ClickMods {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

export interface Selection {
  selected: ReadonlySet<string>;
  size: number;
  has: (id: string) => boolean;
  toggle: (id: string) => void;
  /** Bereich vom zuletzt gewählten Issue bis `id` hinzufügen. */
  extendTo: (id: string) => void;
  clear: () => void;
  /**
   * Klick auf Zeile/Karte: mit Shift/Cmd/Strg oder bei bestehender Auswahl wird gewählt und `true`
   * geliefert (Navigation unterdrücken), sonst `false`.
   */
  handleClick: (id: string, mods: ClickMods) => boolean;
}

export function useSelection(orderedIds: string[]): Selection {
  const [raw, setRaw] = useState<ReadonlySet<string>>(new Set());
  const anchor = useRef<string | null>(null);
  const present = useMemo(() => new Set(orderedIds), [orderedIds]);
  const selected = useMemo(() => {
    if ([...raw].every((id) => present.has(id))) return raw;
    return new Set([...raw].filter((id) => present.has(id)));
  }, [raw, present]);

  const toggle = useCallback((id: string) => {
    anchor.current = id;
    setRaw((prev) => toggled(prev, id));
  }, []);
  const extendTo = useCallback(
    (id: string) => {
      const range = rangeBetween(orderedIds, anchor.current, id);
      if (anchor.current === null) anchor.current = id;
      setRaw((prev) => new Set([...prev, ...range]));
    },
    [orderedIds],
  );
  const clear = useCallback(() => {
    anchor.current = null;
    setRaw(new Set());
  }, []);
  const size = selected.size;
  const handleClick = useCallback(
    (id: string, mods: ClickMods) => {
      if (mods.shiftKey) {
        extendTo(id);
        return true;
      }
      if (mods.metaKey || mods.ctrlKey || size > 0) {
        toggle(id);
        return true;
      }
      return false;
    },
    [extendTo, toggle, size],
  );
  return useMemo(
    () => ({ selected, size, has: (id) => selected.has(id), toggle, extendTo, clear, handleClick }),
    [selected, size, toggle, extendTo, clear, handleClick],
  );
}
