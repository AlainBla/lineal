/**
 * Bekannte Personen für Assignee-Picker und @-Mentions: das Verzeichnis kommt aus `users(query, team, first)`
 * (ohne E-Mail), ergänzt um Autoren gesehener Kommentare und den angemeldeten User.
 */
import { useQuery } from "@tanstack/react-query";
import { useEffect, useSyncExternalStore } from "react";

import type { Comment } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

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

export interface DirectoryUser {
  id: string;
  name: string;
  displayName: string;
}

/** Sichtbare Personen (`users`-Abfrage); gemeinsamer Query-Key `["users", query, team]`. */
export async function fetchUsers(
  query: string,
  team: string | undefined,
  first = 200,
  signal?: AbortSignal,
): Promise<DirectoryUser[]> {
  const res = await gqlRequest<{ users: DirectoryUser[] }, Record<string, unknown>>(
    `query KnownUsers($query: String, $team: String, $first: Int!) {
      users(query: $query, team: $team, first: $first) { id name displayName }
    }`,
    { query: query || undefined, team, first },
    { signal },
  );
  return res.users;
}

/** Lädt das Personenverzeichnis einmal und füllt damit den Speicher der bekannten Personen. */
function useDirectory(): void {
  const q = useQuery({
    queryKey: ["users", "", ""] as const,
    staleTime: 60_000,
    queryFn: ({ signal }) => fetchUsers("", undefined, 200, signal),
  });
  useEffect(() => {
    if (q.data) rememberUsers(q.data.map((u) => ({ id: u.id, name: u.displayName || u.name })));
  }, [q.data]);
}

export function useKnownUsers(): KnownUser[] {
  useDirectory();
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
