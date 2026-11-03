/**
 * Zustand des Issue-Kontextmenüs (Rechtsklick / Kontextmenü-Taste / ⇧F10 auf Zeile oder Karte).
 * Zeilen und Karten melden nur Ziel und Position; das Menü selbst hängt einmal im `IssuesHost`.
 */
import { useSyncExternalStore } from "react";

import type { Issue } from "@/gql/graphql";

import type { Selection } from "./selection";

export interface IssueMenuRequest {
  /** Issue unter dem Zeiger bzw. mit Fokus. */
  issue: Issue;
  /** Betroffene Issues: bei Rechtsklick in die Mehrfachauswahl alle markierten, sonst nur `issue`. */
  targets: Issue[];
  x: number;
  y: number;
  /** Leert die Mehrfachauswahl nach einer erfolgreichen Sammelaktion. */
  clearSelection?: () => void;
}

let current: IssueMenuRequest | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function openIssueMenu(req: IssueMenuRequest): void {
  current = req;
  emit();
}

export function closeIssueMenu(): void {
  if (current === null) return;
  current = null;
  emit();
}

export function useIssueMenuRequest(): IssueMenuRequest | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
    () => current,
  );
}

/** Ziele eines Menüs: in der Mehrfachauswahl enthalten → alle markierten Issues, sonst nur dieses. */
export function menuTargets(issue: Issue, selection?: Selection): Issue[] {
  if (selection && selection.size > 1 && selection.has(issue.id) && selection.issues.length > 1)
    return [...selection.issues];
  return [issue];
}

/**
 * Handler für Zeile/Karte: `onContextMenu` (Rechtsklick) und `onKeyDown` (Kontextmenü-Taste,
 * ⇧F10) öffnen das Menü; per Tastatur erscheint es am unteren Rand des Elements.
 */
export function issueMenuHandlers(issue: Issue, selection?: Selection) {
  const open = (x: number, y: number) =>
    openIssueMenu({
      issue,
      targets: menuTargets(issue, selection),
      x,
      y,
      clearSelection: selection?.clear,
    });
  const atElement = (el: Element) => {
    const r = el.getBoundingClientRect();
    open(r.left + 16, r.bottom - 4);
  };
  return {
    onContextMenu: (e: {
      preventDefault: () => void;
      clientX: number;
      clientY: number;
      currentTarget: Element;
    }) => {
      e.preventDefault();
      // Tastaturausgelöstes contextmenu-Ereignis hat keine Zeigerposition (0/0).
      if (e.clientX === 0 && e.clientY === 0) atElement(e.currentTarget);
      else open(e.clientX, e.clientY);
    },
    onKeyDown: (e: {
      key: string;
      shiftKey: boolean;
      ctrlKey: boolean;
      metaKey: boolean;
      altKey: boolean;
      target: EventTarget;
      currentTarget: Element;
      preventDefault: () => void;
      stopPropagation: () => void;
    }) => {
      // Nur wenn die Zeile selbst den Fokus hat (nicht Checkbox o. Ä. darin).
      if (e.target !== e.currentTarget) return;
      const noMods = !e.ctrlKey && !e.metaKey && !e.altKey;
      const isKey = noMods && (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey));
      if (!isKey) return;
      e.preventDefault();
      e.stopPropagation();
      atElement(e.currentTarget);
    },
  };
}
