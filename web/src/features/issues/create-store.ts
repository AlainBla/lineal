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
