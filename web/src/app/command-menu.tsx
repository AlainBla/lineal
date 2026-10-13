import { useRouter } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import { groupBySection, useCommands, type Command } from "@/app/commands";
import {
  parseCommandQuery,
  PREFIXES,
  useObjectHits,
  type ObjectHit,
  type ParsedQuery,
} from "@/app/command-search";
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

/**
 * Command-Menü (cmdk) mit Registry-Einträgen; `Ctrl/Cmd+K` öffnet es. Mit Präfix (`i `, `p `, `u `,
 * `t `, `l `, `f `, `d `, `n `) zeigt es nur Objekttreffer dieser Art, sonst Befehle und darunter
 * „Quick results“ (docs/05 §5.6). Treffer liefert der Aufrufer (`hits`), damit das Menü ohne
 * Datenzugriff testbar bleibt.
 */
export function CommandMenu({
  open,
  onOpenChange,
  initialQuery = "",
  onQueryChange,
  hits = [],
  hitsLoading = false,
  onPickHit,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  initialQuery?: string;
  onQueryChange?: (q: string) => void;
  hits?: ObjectHit[];
  hitsLoading?: boolean;
  onPickHit?: (h: ObjectHit) => void;
}) {
  const cmds = useCommands();
  const groups = useMemo(() => groupBySection(cmds), [cmds]);
  const [query, setQuery] = useState(initialQuery);
  useShortcutContext("modal", open);
  // Beim Öffnen mit der Vorgabe starten (Open issue… setzt `i `).
  useEffect(() => {
    if (open) setQuery(initialQuery);
  }, [open, initialQuery]);
  const parsed: ParsedQuery = parseCommandQuery(query);
  const prefixInfo = parsed.prefix ? PREFIXES[parsed.prefix] : null;
  const change = (q: string) => {
    setQuery(q);
    onQueryChange?.(q);
  };

  const run = (c: Command) => {
    onOpenChange(false);
    // Nach dem Schließen ausführen, damit Fokus/Overlays der Aktion nicht kollidieren.
    queueMicrotask(c.perform);
  };
  const pick = (h: ObjectHit) => {
    onOpenChange(false);
    queueMicrotask(() => onPickHit?.(h));
  };
  const hitGroups = useMemo(() => {
    const m = new Map<string, ObjectHit[]>();
    for (const h of hits) m.set(h.kind, [...(m.get(h.kind) ?? []), h]);
    return [...m.entries()];
  }, [hits]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent position="top" className="max-w-xl" aria-describedby="command-menu-desc">
        <DialogTitle className="sr-only">Command menu</DialogTitle>
        <DialogDescription id="command-menu-desc" className="sr-only">
          Search for a command or page and press Enter to run it. Start with i, p, u, t, l, f, d or
          n and a space to search issues, projects, users, teams, labels, favorites, documents or
          initiatives.
        </DialogDescription>
        <CommandRoot loop label="Command menu" shouldFilter={!prefixInfo}>
          <CommandInput
            placeholder={prefixInfo?.placeholder ?? "Type a command or search…"}
            value={query}
            onValueChange={change}
          />
          <CommandList>
            <CommandEmpty>{hitsLoading ? "Searching…" : "No results found."}</CommandEmpty>
            {!prefixInfo &&
              groups.map(([section, items]) => (
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
            {hitGroups.map(([kind, items]) => (
              <CommandGroup
                key={kind}
                heading={prefixInfo ? PREFIXES[parsed.prefix!].label : "Quick results"}
                forceMount
              >
                {items.map((h) => (
                  <CommandItem
                    key={h.id}
                    value={`result ${h.id} ${h.title}`}
                    forceMount
                    onSelect={() => pick(h)}
                  >
                    <span className="size-4 shrink-0 text-[10px] text-muted-foreground uppercase">
                      {h.kind.slice(0, 1)}
                    </span>
                    <span className="truncate">{h.title}</span>
                    {h.hint && (
                      <span className="ml-auto shrink-0 text-xs text-subtle-foreground">
                        {h.hint}
                      </span>
                    )}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
        </CommandRoot>
      </DialogContent>
    </Dialog>
  );
}

/** Menü mit Objektsuche; wird nur gemountet, solange es offen ist (keine Abfragen im Ruhezustand). */
function MenuWithSearch({ initialQuery }: { initialQuery: string }) {
  const router = useRouter();
  const [query, setQuery] = useState(initialQuery);
  const { hits, loading } = useObjectHits(parseCommandQuery(query));
  return (
    <CommandMenu
      open
      onOpenChange={(o) => toggleCommandMenu(o)}
      initialQuery={initialQuery}
      onQueryChange={setQuery}
      hits={hits}
      hitsLoading={loading}
      onPickHit={(h) => router.history.push(h.to)}
    />
  );
}

/** Verbunden mit dem globalen UI-Zustand. */
export function ConnectedCommandMenu() {
  const { commandMenuOpen, commandMenuQuery } = useUiState();
  if (!commandMenuOpen) {
    return <CommandMenu open={false} onOpenChange={(o) => toggleCommandMenu(o)} />;
  }
  return <MenuWithSearch initialQuery={commandMenuQuery} />;
}
