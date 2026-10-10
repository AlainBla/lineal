/** Tastaturfokus in Liste und Board (J/K, Pfeile): bewegt den DOM-Fokus auf Zeilen/Karten. */
import { useCallback } from "react";

import { useShortcutContext, useShortcuts } from "@/lib/shortcuts";

import { nextFocus, type FocusDir } from "./model";

export function useFocusNav(
  columns: string[][],
  focusedId: string | null,
  setFocusedId: (id: string | null) => void,
  enabled = true,
): void {
  useShortcutContext("list", enabled);
  const move = useCallback(
    (dir: FocusDir) => {
      const id = nextFocus(columns, focusedId, dir);
      if (!id) return false as const;
      setFocusedId(id);
      document.querySelector<HTMLElement>(`[data-issue-id="${id}"]`)?.focus();
    },
    [columns, focusedId, setFocusedId],
  );
  useShortcuts([
    {
      id: "list.down",
      keys: "j",
      context: "list",
      description: "Focus next",
      run: () => move("down"),
    },
    {
      id: "list.up",
      keys: "k",
      context: "list",
      description: "Focus previous",
      run: () => move("up"),
    },
    {
      id: "list.down-arrow",
      keys: "arrowdown",
      context: "list",
      description: "Focus next",
      run: () => move("down"),
    },
    {
      id: "list.up-arrow",
      keys: "arrowup",
      context: "list",
      description: "Focus previous",
      run: () => move("up"),
    },
    {
      id: "list.left",
      keys: "arrowleft",
      context: "list",
      description: "Focus left",
      run: () => move("left"),
    },
    {
      id: "list.right",
      keys: "arrowright",
      context: "list",
      description: "Focus right",
      run: () => move("right"),
    },
  ]);
}
