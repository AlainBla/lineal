import { useMemo } from "react";

import { groupBySection, useCommands, type Command } from "@/app/commands";
import { toggleCommandMenu, useUiState } from "@/app/ui-state";
import {
  Command as CommandRoot,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Keys } from "@/components/ui/kbd";
import { useShortcutContext } from "@/lib/shortcuts";

/** Command-Menü (cmdk) mit Registry-Einträgen; `Ctrl/Cmd+K` öffnet es. */
export function CommandMenu({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
}) {
  const cmds = useCommands();
  const groups = useMemo(() => groupBySection(cmds), [cmds]);
  useShortcutContext("modal", open);

  const run = (c: Command) => {
    onOpenChange(false);
    // Nach dem Schließen ausführen, damit Fokus/Overlays der Aktion nicht kollidieren.
    queueMicrotask(c.perform);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent position="top" className="max-w-xl" aria-describedby="command-menu-desc">
        <DialogTitle className="sr-only">Command menu</DialogTitle>
        <DialogDescription id="command-menu-desc" className="sr-only">
          Search for a command or page and press Enter to run it.
        </DialogDescription>
        <CommandRoot loop label="Command menu">
          <CommandInput placeholder="Type a command or search…" />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>
            {groups.map(([section, items]) => (
              <CommandGroup key={section} heading={section}>
                {items.map((c) => {
                  const Icon = c.icon;
                  return (
                    <CommandItem
                      key={c.id}
                      value={`${c.section} ${c.title}`}
                      keywords={c.keywords}
                      onSelect={() => run(c)}
                    >
                      {Icon ? <Icon aria-hidden /> : <span className="size-4" aria-hidden />}
                      <span className="truncate">{c.title}</span>
                      {c.shortcut && <Keys keys={c.shortcut} className="ml-auto" />}
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            ))}
          </CommandList>
        </CommandRoot>
      </DialogContent>
    </Dialog>
  );
}

/** Verbunden mit dem globalen UI-Zustand. */
export function ConnectedCommandMenu() {
  const { commandMenuOpen } = useUiState();
  return <CommandMenu open={commandMenuOpen} onOpenChange={(o) => toggleCommandMenu(o)} />;
}
