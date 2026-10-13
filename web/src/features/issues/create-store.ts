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
 * Entwurf des Create-Modals (Titel/Beschreibung). Er überlebt das Schließen durch Session-Ablauf
 * (Rücksprung zum Login) und wird nach erneuter Anmeldung wieder geöffnet; erfolgreiches Anlegen
 * oder bewusstes Verwerfen löscht ihn.
 */
export interface CreateDraft {
  title: string;
  description: string;
}

const DRAFT_KEY = "lineal-create-draft";
const REOPEN_KEY = "lineal-create-reopen";

function storage(): Storage | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

export function getCreateDraft(): CreateDraft | null {
  const raw = storage()?.getItem(DRAFT_KEY);
  if (!raw) return null;
  try {
    const d = JSON.parse(raw) as Partial<CreateDraft>;
    return { title: String(d.title ?? ""), description: String(d.description ?? "") };
  } catch {
    return null;
  }
}

export function hasCreateDraft(): boolean {
  const d = getCreateDraft();
  return !!d && (d.title.trim() !== "" || d.description.trim() !== "");
}

export function saveCreateDraft(d: CreateDraft): void {
  if (d.title === "" && d.description === "") clearCreateDraft();
  else storage()?.setItem(DRAFT_KEY, JSON.stringify(d));
}

export function clearCreateDraft(): void {
  storage()?.removeItem(DRAFT_KEY);
}

/** Session abgelaufen: nach dem erneuten Login das Modal mit dem Entwurf öffnen. */
export function markCreateReopen(): void {
  if (hasCreateDraft()) storage()?.setItem(REOPEN_KEY, "1");
}

/** `true` genau einmal nach `markCreateReopen`. */
export function consumeCreateReopen(): boolean {
  const s = storage();
  if (!s || s.getItem(REOPEN_KEY) !== "1") return false;
  s.removeItem(REOPEN_KEY);
  return hasCreateDraft();
}
