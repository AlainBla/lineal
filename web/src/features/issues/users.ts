/**
 * Bekannte Personen für Assignee-Picker und @-Mentions. Das GraphQL-Schema hat (noch) keine
 * User-/Mitgliederabfrage; bekannt sind deshalb der angemeldete User und Autoren gesehener Kommentare.
 */
import { useSyncExternalStore } from "react";

import type { Comment } from "@/gql/graphql";

export interface KnownUser {
  id: string;
  name: string;
}

const known = new Map<string, KnownUser>();
let snapshot: KnownUser[] = [];
const listeners = new Set<() => void>();

export function rememberUsers(users: KnownUser[]): void {
  let changed = false;
  for (const u of users) {
    const prev = known.get(u.id);
    if (!prev || prev.name !== u.name) {
      known.set(u.id, u);
      changed = true;
    }
  }
  if (!changed) return;
  snapshot = [...known.values()].sort((a, b) => a.name.localeCompare(b.name));
  listeners.forEach((l) => l());
}

export function rememberCommentAuthors(comments: Comment[]): void {
  rememberUsers(
    comments.flatMap((c) =>
      c.user ? [{ id: c.user.id, name: c.user.displayName || c.user.name }] : [],
    ),
  );
}

export function resetKnownUsers(): void {
  known.clear();
  snapshot = [];
  listeners.forEach((l) => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useKnownUsers(): KnownUser[] {
  return useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => snapshot,
  );
}

/** Anzeigename zu einer User-ID (`me` = angemeldeter User, sonst bekannte Person oder `null`). */
export function userName(
  id: string | null,
  users: KnownUser[],
  me?: { id: string; name: string } | null,
): string | null {
  if (!id) return null;
  if (me && id === me.id) return me.name;
  return users.find((u) => u.id === id)?.name ?? null;
}
