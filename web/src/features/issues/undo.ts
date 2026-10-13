/**
 * Undo/Redo der letzten Feldänderungen (docs/08 §3.2 `Ctrl Z` / `Ctrl ⇧ Z`, F.12). Aktionen legen
 * über `recordUndo` ihre Gegenaktion ab; die Kürzel arbeiten den Stapel ab. Der Stapel gilt je
 * Browser-Sitzung und wird beim Abmelden geleert.
 */
import type { Issue } from "@/gql/graphql";

export interface UndoEntry {
  label: string;
  undo: () => void;
  redo: () => void;
}

const MAX_ENTRIES = 50;
let undos: UndoEntry[] = [];
let redos: UndoEntry[] = [];

export function recordUndo(entry: UndoEntry): void {
  undos = [...undos, entry].slice(-MAX_ENTRIES);
  redos = [];
}

export function undoLast(): UndoEntry | null {
  const e = undos.at(-1);
  if (!e) return null;
  undos = undos.slice(0, -1);
  redos = [...redos, e];
  e.undo();
  return e;
}

export function redoLast(): UndoEntry | null {
  const e = redos.at(-1);
  if (!e) return null;
  redos = redos.slice(0, -1);
  undos = [...undos, e];
  e.redo();
  return e;
}

export function clearUndo(): void {
  undos = [];
  redos = [];
}

/** Felder, deren Änderung rückgängig gemacht werden kann (Titel und Beschreibung haben den Editor-Verlauf). */
const UNDOABLE = [
  "stateId",
  "assigneeId",
  "priority",
  "estimate",
  "parentId",
  "labelIds",
  "projectId",
  "cycleId",
  "dueDate",
  "sortOrder",
] as const;

/** Vorherige Werte der in `input` gesetzten Felder; `null`, wenn nichts davon rückgängig zu machen ist. */
export function inverseInput(
  issue: Issue,
  input: Record<string, unknown>,
): Record<string, unknown> | null {
  const out: Record<string, unknown> = {};
  for (const key of UNDOABLE) {
    if (key in input) out[key] = (issue as unknown as Record<string, unknown>)[key] ?? null;
  }
  return Object.keys(out).length === 0 ? null : out;
}

/** Gegenaktionen einer Bulk-Änderung je Issue; `null`, wenn sie nicht umkehrbar ist (Team, Archiv, Löschen). */
export function inverseBulk(
  issues: Issue[],
  vars: Record<string, unknown>,
): Array<{ id: string; input: Record<string, unknown> }> | null {
  if ("teamId" in vars || vars.archive === true || vars.delete === true) return null;
  const labels = "addedLabelIds" in vars || "removedLabelIds" in vars;
  const out: Array<{ id: string; input: Record<string, unknown> }> = [];
  for (const i of issues) {
    const input = inverseInput(i, vars) ?? {};
    if (labels) input.labelIds = i.labelIds;
    if (Object.keys(input).length > 0) out.push({ id: i.id, input });
  }
  return out.length > 0 ? out : null;
}
