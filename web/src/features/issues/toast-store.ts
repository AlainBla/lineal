/** Minimaler Toast-Speicher (docs/08 §1.4: Toast mit Link und Undo). */
import { useSyncExternalStore } from "react";

export interface Toast {
  id: number;
  message: string;
  tone: "info" | "error";
  /** Interner Pfad für „Open“. */
  href?: string;
  onUndo?: () => void;
}

let toasts: Toast[] = [];
let nextId = 1;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function showToast(t: Omit<Toast, "id" | "tone"> & { tone?: Toast["tone"] }): number {
  const toast: Toast = { tone: "info", ...t, id: nextId++ };
  toasts = [...toasts, toast].slice(-4);
  emit();
  return toast.id;
}

export function dismissToast(id: number): void {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export function clearToasts(): void {
  toasts = [];
  emit();
}

export function errorMessage(err: unknown, fallback = "Something went wrong."): string {
  return err instanceof Error && err.message ? err.message : fallback;
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(
    subscribe,
    () => toasts,
    () => toasts,
  );
}
