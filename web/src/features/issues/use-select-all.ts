/** `Ctrl A` wählt alle Issues der Ansicht (docs/08 §3.2); in Texteingaben bleibt es beim Browser-Verhalten. */
import { useActions } from "@/app/commands";
import { useShortcuts } from "@/lib/shortcuts";

import { isEditingText } from "./global-shortcuts";
import type { Selection } from "./selection";

export function useSelectAll(selection: Selection, enabled: boolean): void {
  const run = () => selection.selectAll();
  useShortcuts([
    {
      id: "issue.select-all",
      keys: "mod+a",
      context: "list",
      description: "Select all issues",
      run: () => (!enabled || isEditingText() ? false : run()),
    },
  ]);
  useActions([
    {
      id: "issue.select-all.cmd",
      title: "Select all issues",
      section: "Issues",
      shortcut: "mod+a",
      enabled,
      perform: run,
    },
  ]);
}
