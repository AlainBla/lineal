import { useMemo, useState } from "react";

import { hiddenStateIds, splitBoardColumns, type BoardColumn, type HiddenColumn } from "./model";

export interface BoardVisibility {
  shown: BoardColumn[];
  hidden: HiddenColumn[];
  /** Spalte für diese Sitzung ein- oder ausblenden (Klick unter „Hidden columns“). */
  toggle: (stateId: string) => void;
  /** Ist der Status dauerhaft versteckt (auch wenn gerade eingeblendet)? */
  isHidden: (stateId: string) => boolean;
  /** Dauerhaft verstecken; fehlt, wo nichts gespeichert wird. */
  hide?: (stateId: string) => void;
  /** Dauerhaftes Verstecken aufheben; fehlt, wo nichts gespeichert wird. */
  keep?: (stateId: string) => void;
}

/**
 * Welche Board-Spalten sichtbar sind. Die Seite braucht das Ergebnis auch selbst: Tastatur-Fokus
 * und Mehrfachauswahl dürfen nur sichtbare Karten erreichen.
 */
export function useBoardVisibility(
  columns: BoardColumn[],
  prefs?: {
    hiddenGroups: string[] | null;
    hideEmpty: boolean;
    onHiddenGroups: (ids: string[]) => void;
  },
): BoardVisibility {
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set());
  const hiddenGroups = prefs?.hiddenGroups ?? null;
  const hideEmpty = prefs?.hideEmpty ?? false;
  const { shown, hidden } = useMemo(
    () => splitBoardColumns(columns, { hiddenGroups, hideEmpty, revealed }),
    [columns, hiddenGroups, hideEmpty, revealed],
  );
  const manual = hiddenStateIds(columns, hiddenGroups);
  const save = prefs?.onHiddenGroups;
  return {
    shown,
    hidden,
    toggle: (id) =>
      setRevealed((prev) => {
        const next = new Set(prev);
        if (!next.delete(id)) next.add(id);
        return next;
      }),
    isHidden: (id) => manual.has(id),
    hide: save
      ? (id) => {
          setRevealed((prev) => new Set([...prev].filter((r) => r !== id)));
          save([...manual, id]);
        }
      : undefined,
    keep: save ? (id) => save([...manual].filter((m) => m !== id)) : undefined,
  };
}
