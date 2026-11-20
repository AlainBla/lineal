import { useState, type ReactNode } from "react";

import { useCommands } from "@/app/commands";
import { toggleShortcutHelp, useUiState } from "@/app/ui-state";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Kbd, Keys } from "@/components/ui/kbd";
import { useShortcutContext } from "@/lib/shortcuts";

/**
 * Editor-Kürzel (Tiptap/StarterKit + eigene Extensions, `rich-editor.tsx`/`emoji-input.ts`, LIN-78).
 * Diese sind fix und stehen nicht in der Command-Registry, daher statisch gepflegt.
 */
const EDITOR_KEYS: Array<{ label: string; keys?: string; char?: string }> = [
  { label: "Comment absenden", keys: "mod+enter" },
  { label: "Mention einfügen", char: "@" },
  { label: "Emoji einfügen", char: ":" },
  { label: "Fett", keys: "mod+b" },
  { label: "Kursiv", keys: "mod+i" },
  { label: "Inline-Code", keys: "mod+e" },
  { label: "Durchgestrichen", keys: "mod+shift+s" },
  { label: "Überschrift 1–6", keys: "mod+alt+1" },
  { label: "Aufzählung", keys: "mod+shift+8" },
  { label: "Nummerierung", keys: "mod+shift+7" },
  { label: "Aufgabenliste", keys: "mod+shift+9" },
  { label: "Zitat", keys: "mod+shift+b" },
  { label: "Codeblock", keys: "mod+alt+c" },
  { label: "Zeilenumbruch", keys: "shift+enter" },
  { label: "Rückgängig", keys: "mod+z" },
  { label: "Wiederholen", keys: "mod+shift+z" },
];

/** Markdown-Eingaberegeln: tippen wandelt um. */
const EDITOR_MARKDOWN: Array<{ label: string; md: string }> = [
  { label: "Fett", md: "**text**" },
  { label: "Kursiv", md: "*text*" },
  { label: "Durchgestrichen", md: "~~text~~" },
  { label: "Inline-Code", md: "`code`" },
  { label: "Überschrift", md: "# " },
  { label: "Zitat", md: "> " },
  { label: "Aufzählung", md: "- " },
  { label: "Nummerierung", md: "1. " },
  { label: "Aufgabe", md: "[ ] " },
  { label: "Codeblock", md: "``` " },
  { label: "Trennlinie", md: "---" },
];

/** Kürzel-Übersicht (`Ctrl /`): App-Commands plus die Editor-Kürzel, durchsuchbar. */
export function ShortcutHelp() {
  const { shortcutHelpOpen } = useUiState();
  const [query, setQuery] = useState("");
  const cmds = useCommands();
  useShortcutContext("modal", shortcutHelpOpen);

  const q = query.trim().toLowerCase();
  const match = (section: string, label: string) =>
    !q || `${section} ${label}`.toLowerCase().includes(q);

  const rows = cmds
    .filter((c) => c.shortcut)
    .filter((c) => !q || `${c.section} ${c.title}`.toLowerCase().includes(q));
  const editor = EDITOR_KEYS.filter((e) => match("Editor", e.label));
  const markdown = EDITOR_MARKDOWN.filter((e) => match("Editor Markdown", e.label));
  const empty = rows.length === 0 && editor.length === 0 && markdown.length === 0;

  const row = (key: string, label: string, right: ReactNode, section?: string) => (
    <li
      key={key}
      className="flex items-center justify-between gap-4 rounded-md px-2 py-1.5 hover:bg-surface-hover"
    >
      <span>
        {section && <span className="mr-2 text-xs text-subtle-foreground">{section}</span>}
        {label}
      </span>
      {right}
    </li>
  );
  const heading = (text: string) => (
    <li className="px-2 pt-3 pb-1 text-xs font-semibold text-subtle-foreground">{text}</li>
  );

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
          {empty && <li className="py-6 text-center text-muted-foreground">No shortcuts found.</li>}
          {rows.map((c) =>
            row(c.id, c.title, c.shortcut ? <Keys keys={c.shortcut} /> : null, c.section),
          )}
          {editor.length > 0 && heading("Editor")}
          {editor.map((e) =>
            row(`ed-${e.label}`, e.label, e.keys ? <Keys keys={e.keys} /> : <Kbd>{e.char}</Kbd>),
          )}
          {markdown.length > 0 && heading("Editor · Markdown-Regeln")}
          {markdown.map((e) => row(`md-${e.label}`, e.label, <Kbd>{e.md}</Kbd>))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
