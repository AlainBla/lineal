/**
 * Globale Kürzel rund um Undo und Toasts (docs/08 §3.2, F.12): `Ctrl Z` / `Ctrl ⇧ Z` Undo und Redo der
 * letzten Feldänderung, `Ctrl Alt O` öffnet den Link des jüngsten Toasts, `Ctrl Alt X` schließt alle
 * Toasts. In Eingabefeldern und Editoren bleibt `Ctrl Z` beim Browser-Undo des Textes.
 */
import { useRouter } from "@tanstack/react-router";

import { useActions } from "@/app/commands";
import { useShortcuts } from "@/lib/shortcuts";

import { clearToasts, getToasts, showToast } from "./toast-store";
import { redoLast, undoLast } from "./undo";

export function isEditingText(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.tagName === "SELECT" ||
    el.isContentEditable === true
  );
}

export function useIssueGlobalShortcuts(): void {
  const router = useRouter();
  const undo = () => {
    const e = undoLast();
    if (!e) return false as const;
    showToast({ message: `Undid ${e.label}` });
  };
  const redo = () => {
    const e = redoLast();
    if (!e) return false as const;
    showToast({ message: `Redid ${e.label}` });
  };
  const openToastLink = () => {
    const href = [...getToasts()].reverse().find((t) => t.href)?.href;
    if (!href) return false as const;
    router.history.push(href);
  };
  useShortcuts([
    {
      id: "undo.last",
      keys: "mod+z",
      description: "Undo last action",
      run: () => (isEditingText() ? false : undo()),
    },
    {
      id: "redo.last",
      keys: "mod+shift+z",
      description: "Redo last action",
      run: () => (isEditingText() ? false : redo()),
    },
    {
      id: "toast.open",
      keys: "mod+alt+o",
      description: "Open link from last toast",
      run: openToastLink,
    },
    { id: "toast.clear", keys: "mod+alt+x", description: "Dismiss all toasts", run: clearToasts },
  ]);
  // Im Command-Menü auffindbar; die Tasten selbst stehen oben (mit Prüfung auf Texteingabe).
  useActions([
    {
      id: "undo.cmd",
      title: "Undo last action",
      section: "General",
      shortcut: "mod+z",
      perform: undo,
    },
    {
      id: "redo.cmd",
      title: "Redo last action",
      section: "General",
      shortcut: "mod+shift+z",
      perform: redo,
    },
    {
      id: "toast.clear.cmd",
      title: "Dismiss all toasts",
      section: "General",
      shortcut: "mod+alt+x",
      perform: clearToasts,
    },
  ]);
}
