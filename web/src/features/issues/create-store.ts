/**
 * Vorbelegung des Create-Issue-Modals (docs/08 §2.3 „Kontext-Vorbelegung“). `c` und der Stift in der
 * Sidebar setzen nur `createIssueOpen`; „+“ in Gruppen-/Spaltenköpfen ruft `openCreateIssue` mit Werten.
 */
import { useMemo, useSyncExternalStore } from "react";

import { setUiState } from "@/app/ui-state";

export interface CreatePreset {
  teamKey?: string;
  stateId?: string;
  assigneeId?: string;
  priority?: number;
  projectId?: string;
  cycleId?: string;
  parentId?: string;
  /** Alt+C: das Modal öffnet mit der Template-Auswahl. */
  pickTemplate?: boolean;
}

let preset: CreatePreset | null = null;
/** Kontext der aktuellen Seite (Projekt, Cycle …): Vorbelegung für `c`/Stift ohne eigenes Preset. */
let context: CreatePreset | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function openCreateIssue(p: CreatePreset = {}): void {
  preset = p;
  emit();
  setUiState({ createIssueOpen: true });
}

/** Alt+C: Modal mit Kontext-Vorbelegung öffnen und direkt die Template-Auswahl zeigen. */
export function openCreateFromTemplate(): void {
  openCreateIssue({ ...context, pickTemplate: true });
}

/** Seiten melden ihren Kontext, solange sie offen sind; beim Verlassen mit `null` zurücknehmen. */
export function setCreateContext(c: CreatePreset | null): void {
  context = c;
  emit();
}

export function clearCreatePreset(): void {
  if (preset === null) return;
  preset = null;
  emit();
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useCreatePreset(): CreatePreset | null {
  const get = () => preset ?? context;
  return useSyncExternalStore(subscribe, get, get);
}

/**
 * Entwurf des Create-Modals (Titel/Beschreibung). Er ist strikt an Nutzer und Workspace gebunden
 * (`scope`), liegt nur in `sessionStorage` (nie localStorage, endet mit dem Tab) und überlebt
 * Session-Ablauf (Rücksprung zum Login), damit derselbe Nutzer nach dem erneuten Login
 * weiterschreiben kann. Beim Abmelden werden alle Entwürfe gelöscht; beim Anmelden verwirft
 * `purgeForeignDrafts` Entwürfe anderer Nutzer/Workspaces. Erfolgreiches Anlegen oder bewusstes
 * Verwerfen löscht den Entwurf.
 */
export interface CreateDraft {
  title: string;
  description: string;
}

/** Schlüssel aus Nutzer-ID und Workspace; `null`, solange keine Session bekannt ist. */
export type DraftScope = string;
export const draftScope = (userId: string, workspaceId: string): DraftScope =>
  `${userId}:${workspaceId}`;

const DRAFT_PREFIX = "lineal-create-draft:";
const LIST_PREFIX = "lineal-create-drafts:";
const REOPEN_PREFIX = "lineal-create-reopen:";

function storage(): Storage | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

export function getCreateDraft(scope: DraftScope): CreateDraft | null {
  const raw = storage()?.getItem(DRAFT_PREFIX + scope);
  if (!raw) return null;
  try {
    const d = JSON.parse(raw) as Partial<CreateDraft>;
    return { title: String(d.title ?? ""), description: String(d.description ?? "") };
  } catch {
    return null;
  }
}

export function hasCreateDraft(scope: DraftScope): boolean {
  const d = getCreateDraft(scope);
  return !!d && (d.title.trim() !== "" || d.description.trim() !== "");
}

export function saveCreateDraft(scope: DraftScope, d: CreateDraft): void {
  if (d.title === "" && d.description === "") clearCreateDraft(scope);
  else storage()?.setItem(DRAFT_PREFIX + scope, JSON.stringify(d));
}

export function clearCreateDraft(scope: DraftScope): void {
  storage()?.removeItem(DRAFT_PREFIX + scope);
  storage()?.removeItem(REOPEN_PREFIX + scope);
}

/** Abmelden: alle Entwürfe und Wiederöffnen-Marken entfernen. */
export function clearAllCreateDrafts(): void {
  const s = storage();
  if (!s) return;
  for (const k of Object.keys(s)) {
    if (k.startsWith(DRAFT_PREFIX) || k.startsWith(REOPEN_PREFIX) || k.startsWith(LIST_PREFIX)) {
      s.removeItem(k);
    }
  }
  emit();
}

/** Anmelden: Entwürfe anderer Nutzer/Workspaces löschen, nur der eigene Scope bleibt. */
export function purgeForeignDrafts(scope: DraftScope): void {
  const s = storage();
  if (!s) return;
  for (const k of Object.keys(s)) {
    const own =
      k === DRAFT_PREFIX + scope || k === REOPEN_PREFIX + scope || k === LIST_PREFIX + scope;
    const draftKey =
      k.startsWith(DRAFT_PREFIX) || k.startsWith(REOPEN_PREFIX) || k.startsWith(LIST_PREFIX);
    if (!own && draftKey) s.removeItem(k);
  }
  emit();
}

/** Session abgelaufen: nach dem erneuten Login desselben Nutzers das Modal wieder öffnen. */
export function markCreateReopen(scope: DraftScope): void {
  if (hasCreateDraft(scope)) storage()?.setItem(REOPEN_PREFIX + scope, "1");
}

/** `true` genau einmal nach `markCreateReopen`, nur für denselben Scope. */
export function consumeCreateReopen(scope: DraftScope): boolean {
  const s = storage();
  if (!s || s.getItem(REOPEN_PREFIX + scope) !== "1") return false;
  s.removeItem(REOPEN_PREFIX + scope);
  return hasCreateDraft(scope);
}

/**
 * Gespeicherte Entwürfe („Drafts“, docs/08 §2.3): Wird das Modal mit Inhalt geschlossen, kann der
 * Nutzer ihn als Draft ablegen. Dieselben Regeln wie beim Arbeitsentwurf: an Nutzer und Workspace
 * gebunden (`scope`), nur `sessionStorage`, beim Abmelden gelöscht, fremde Entwürfe werden beim
 * Anmelden verworfen.
 */
export interface SavedDraft {
  id: string;
  title: string;
  description: string;
  teamKey?: string;
  savedAt: string;
}

function readList(scope: DraftScope): SavedDraft[] {
  const raw = storage()?.getItem(LIST_PREFIX + scope);
  if (!raw) return [];
  try {
    const list = JSON.parse(raw) as unknown;
    if (!Array.isArray(list)) return [];
    return list.flatMap((d): SavedDraft[] => {
      const o = d as Partial<SavedDraft> | null;
      if (!o || typeof o.id !== "string") return [];
      return [
        {
          id: o.id,
          title: String(o.title ?? ""),
          description: String(o.description ?? ""),
          teamKey: typeof o.teamKey === "string" ? o.teamKey : undefined,
          savedAt: typeof o.savedAt === "string" ? o.savedAt : new Date(0).toISOString(),
        },
      ];
    });
  } catch {
    return [];
  }
}

function writeList(scope: DraftScope, list: SavedDraft[]): void {
  if (list.length === 0) storage()?.removeItem(LIST_PREFIX + scope);
  else storage()?.setItem(LIST_PREFIX + scope, JSON.stringify(list));
  emit();
}

export function listSavedDrafts(scope: DraftScope): SavedDraft[] {
  return readList(scope).sort((a, b) => b.savedAt.localeCompare(a.savedAt));
}

export function addSavedDraft(
  scope: DraftScope,
  d: Pick<SavedDraft, "title" | "description" | "teamKey">,
): SavedDraft | null {
  if (d.title.trim() === "" && d.description.trim() === "") return null;
  const draft: SavedDraft = {
    ...d,
    id: crypto.randomUUID(),
    savedAt: new Date().toISOString(),
  };
  writeList(scope, [...readList(scope), draft]);
  return draft;
}

export function removeSavedDraft(scope: DraftScope, id: string): void {
  writeList(
    scope,
    readList(scope).filter((d) => d.id !== id),
  );
}

/** Reaktive Liste der gespeicherten Entwürfe (Drafts-Seite, Sidebar-Zähler). */
export function useSavedDrafts(scope: DraftScope | null): SavedDraft[] {
  const raw = useSyncExternalStore(
    subscribe,
    () => (scope ? (storage()?.getItem(LIST_PREFIX + scope) ?? "") : ""),
    () => "",
  );
  return useMemo(
    () => (scope && raw ? listSavedDrafts(scope) : []),
    // `raw` ändert sich mit jedem Schreibzugriff auf die Liste.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scope, raw],
  );
}
