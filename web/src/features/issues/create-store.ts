/**
 * Vorbelegung des Create-Issue-Modals (docs/08 §2.3 „Kontext-Vorbelegung“). `c` und der Stift in der
 * Sidebar setzen nur `createIssueOpen`; „+“ in Gruppen-/Spaltenköpfen ruft `openCreateIssue` mit Werten.
 */
import { useSyncExternalStore } from "react";

import { setUiState } from "@/app/ui-state";

export interface CreatePreset {
  teamKey?: string;
  stateId?: string;
  assigneeId?: string;
  priority?: number;
  projectId?: string;
  cycleId?: string;
  parentId?: string;
}

let preset: CreatePreset | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function openCreateIssue(p: CreatePreset = {}): void {
  preset = p;
  emit();
  setUiState({ createIssueOpen: true });
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
  return useSyncExternalStore(
    subscribe,
    () => preset,
    () => preset,
  );
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
    if (k.startsWith(DRAFT_PREFIX) || k.startsWith(REOPEN_PREFIX)) s.removeItem(k);
  }
}

/** Anmelden: Entwürfe anderer Nutzer/Workspaces löschen, nur der eigene Scope bleibt. */
export function purgeForeignDrafts(scope: DraftScope): void {
  const s = storage();
  if (!s) return;
  for (const k of Object.keys(s)) {
    const own = k === DRAFT_PREFIX + scope || k === REOPEN_PREFIX + scope;
    if (!own && (k.startsWith(DRAFT_PREFIX) || k.startsWith(REOPEN_PREFIX))) s.removeItem(k);
  }
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
