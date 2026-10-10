/**
 * Command-Registry: alles, was das Command-Menü (Ctrl/Cmd+K) anbietet. Jede Aktion mit Tastenkürzel wird
 * über `useActions` gleichzeitig als Shortcut und als Command registriert (docs/08 §3.1), damit Kürzel
 * im Menü entdeckbar bleiben.
 */
import type { LucideIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";

import {
  shortcuts as defaultShortcuts,
  type ShortcutContext,
  type ShortcutRegistry,
} from "@/lib/shortcuts";

export interface Command {
  /** Eindeutig, z. B. `nav.inbox`, `theme.dark`. */
  id: string;
  title: string;
  /** Gruppe im Menü, z. B. `Navigation`, `Preferences`. */
  section: string;
  keywords?: string[];
  /** Tastenfolge nur zur Anzeige (Notation siehe `lib/shortcuts.ts`). */
  shortcut?: string;
  icon?: LucideIcon;
  perform: () => void;
}

export interface Action extends Command {
  /** Wenn gesetzt, wird zusätzlich ein Shortcut registriert. */
  keys?: string;
  context?: ShortcutContext;
  allowInModal?: boolean;
  /** Nur im Command-Menü sichtbar, wenn wahr (Default: immer). */
  enabled?: boolean;
}

type Listener = () => void;

export class CommandRegistry {
  private commands = new Map<string, Command>();
  private listeners = new Set<Listener>();
  private snapshot: Command[] = [];

  register(cmds: Command[]): () => void {
    for (const c of cmds) this.commands.set(c.id, c);
    this.emit();
    return () => {
      for (const c of cmds) if (this.commands.get(c.id) === c) this.commands.delete(c.id);
      this.emit();
    };
  }

  list(): Command[] {
    return this.snapshot;
  }

  /** Einfache Titelsuche (cmdk filtert in der UI selbst; dies dient Tests und Skripten). */
  search(query: string): Command[] {
    const q = query.trim().toLowerCase();
    if (!q) return this.snapshot;
    return this.snapshot.filter((c) =>
      [c.title, c.section, ...(c.keywords ?? [])].some((s) => s.toLowerCase().includes(q)),
    );
  }

  subscribe(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  private emit() {
    this.snapshot = [...this.commands.values()];
    this.listeners.forEach((l) => l());
  }
}

export const commands = new CommandRegistry();

export function useCommands(reg: CommandRegistry = commands): Command[] {
  return useSyncExternalStore(
    (l) => reg.subscribe(l),
    () => reg.list(),
    () => reg.list(),
  );
}

/**
 * Registriert Aktionen für die Lebensdauer der Komponente als Command (+ Shortcut, falls `keys`).
 * `perform` darf aktuelle Closures nutzen; Registrierung hängt nur an id/keys/Titel.
 */
export function useActions(
  actions: Action[],
  deps: { commands?: CommandRegistry; shortcuts?: ShortcutRegistry } = {},
): void {
  const cmdReg = deps.commands ?? commands;
  const keyReg = deps.shortcuts ?? defaultShortcuts;
  const ref = useRef(actions);
  useLayoutEffect(() => {
    ref.current = actions;
  });
  const signature = actions
    .filter((a) => a.enabled !== false)
    .map((a) => `${a.id}|${a.title}|${a.keys ?? ""}|${a.section}`)
    .join("\n");

  useEffect(() => {
    const active = ref.current.filter((a) => a.enabled !== false);
    const offs: Array<() => void> = [];
    offs.push(
      cmdReg.register(
        active.map((a) => ({
          id: a.id,
          title: a.title,
          section: a.section,
          keywords: a.keywords,
          icon: a.icon,
          shortcut: a.keys ?? a.shortcut,
          perform: () => ref.current.find((x) => x.id === a.id)?.perform(),
        })),
      ),
    );
    for (const a of active) {
      if (!a.keys) continue;
      offs.push(
        keyReg.register({
          id: a.id,
          keys: a.keys,
          context: a.context,
          allowInModal: a.allowInModal,
          description: a.title,
          run: () => ref.current.find((x) => x.id === a.id)?.perform(),
        }),
      );
    }
    return () => offs.forEach((off) => off());
  }, [signature, cmdReg, keyReg]);
}

/** Gruppiert Commands nach Sektion; Reihenfolge = erstes Auftreten. */
export function groupBySection(cmds: Command[]): Array<[string, Command[]]> {
  const map = new Map<string, Command[]>();
  for (const c of cmds) {
    const list = map.get(c.section) ?? [];
    list.push(c);
    map.set(c.section, list);
  }
  return [...map.entries()];
}
