import { useMemo, useState } from "react";

import { useCommands } from "@/app/commands";
import { toggleShortcutHelp, useUiState } from "@/app/ui-state";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Keys } from "@/components/ui/kbd";
import { useShortcutContext } from "@/lib/shortcuts";

/** Kürzel-Übersicht (`Ctrl /`): alle Commands mit Tastenkürzel, durchsuchbar. */
export function ShortcutHelp() {
  const { shortcutHelpOpen } = useUiState();
  const [query, setQuery] = useState("");
  const cmds = useCommands();
  useShortcutContext("modal", shortcutHelpOpen);

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return cmds
      .filter((c) => c.shortcut)
      .filter((c) => !q || `${c.section} ${c.title}`.toLowerCase().includes(q));
  }, [cmds, query]);

  return (
    <Dialog
      open={shortcutHelpOpen}
      onOpenChange={(o) => {
        if (!o) setQuery("");
        toggleShortcutHelp(o);
      }}
    >
      <DialogContent position="top" className="max-w-lg">
        <DialogTitle className="px-4 pt-4 text-sm font-semibold">Keyboard shortcuts</DialogTitle>
        <DialogDescription className="sr-only">All available keyboard shortcuts.</DialogDescription>
        <div className="p-4 pb-2">
          <Input
            placeholder="Search shortcuts…"
            aria-label="Search shortcuts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <ul className="max-h-[50vh] overflow-y-auto px-2 pb-3">
          {rows.length === 0 && (
            <li className="py-6 text-center text-muted-foreground">No shortcuts found.</li>
          )}
          {rows.map((c) => (
            <li
              key={c.id}
              className="flex items-center justify-between gap-4 rounded-md px-2 py-1.5 hover:bg-surface-hover"
            >
              <span>
                <span className="mr-2 text-xs text-subtle-foreground">{c.section}</span>
                {c.title}
              </span>
              {c.shortcut && <Keys keys={c.shortcut} />}
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
