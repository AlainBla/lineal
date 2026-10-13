/**
 * Bekannte Personen für Assignee-Picker und @-Mentions. Das Verzeichnis kommt aus `users(query, team, first)`
 * (ohne E-Mail), ergänzt um Autoren gesehener Kommentare. Es gibt keinen Modulzustand: alles liegt im
 * TanStack-Query-Cache, Schlüssel enthalten User und Workspace der Session, und der Logout leert den Cache
 * (`client.clear()` in der Shell). Damit sieht ein anderer User nie Personen des vorherigen.
 */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";

import { useSession } from "@/app/auth";
import type { Comment } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export interface KnownUser {
  id: string;
  name: string;
}

/** Session-Scope der Cache-Schlüssel (User-ID und Workspace). */
export interface UserScope {
  userId: string;
  ws: string;
}

export const usersKey = (scope: UserScope | null, query = "", team = "") =>
  ["users", scope?.userId ?? "", scope?.ws ?? "", query, team] as const;
const extraKey = (scope: UserScope | null) =>
  ["known-users", scope?.userId ?? "", scope?.ws ?? ""] as const;

export function useUserScope(): UserScope | null {
  const session = useSession();
  return useMemo(
    () => (session ? { userId: session.user.id, ws: session.workspace.urlKey } : null),
    [session],
  );
}

export interface DirectoryUser {
  id: string;
  name: string;
  displayName: string;
}

/** Sichtbare Personen (`users`-Abfrage). */
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

export const toKnown = (users: DirectoryUser[]): KnownUser[] =>
  users.map((u) => ({ id: u.id, name: u.displayName || u.name }));

export function commentAuthors(comments: Comment[]): KnownUser[] {
  return comments.flatMap((c) =>
    c.user ? [{ id: c.user.id, name: c.user.displayName || c.user.name }] : [],
  );
}

/** Merkt zusätzliche Personen (z. B. Kommentar-Autoren) im Session-Cache. */
export function useRememberUsers(): (users: KnownUser[]) => void {
  const qc = useQueryClient();
  const scope = useUserScope();
  return useCallback(
    (users: KnownUser[]) => {
      qc.setQueryData<KnownUser[]>(extraKey(scope), (old = []) => {
        const map = new Map(old.map((u) => [u.id, u]));
        for (const u of users) map.set(u.id, u);
        return [...map.values()];
      });
    },
    [qc, scope],
  );
}

/** Verzeichnis der Session (geteilter Cache mit `useUsers` der Views). */
export function useDirectoryUsers(query = "", team?: string, enabled = true) {
  const scope = useUserScope();
  return useQuery({
    queryKey: usersKey(scope, query, team ?? ""),
    enabled: enabled && !!scope,
    staleTime: 60_000,
    queryFn: ({ signal }) => fetchUsers(query, team, 200, signal),
  });
}

const EMPTY: KnownUser[] = [];

export function useKnownUsers(): KnownUser[] {
  const scope = useUserScope();
  const dir = useDirectoryUsers();
  const extra = useQuery({
    queryKey: extraKey(scope),
    enabled: !!scope,
    staleTime: Infinity,
    queryFn: () => EMPTY,
  });
  return useMemo(() => {
    const map = new Map<string, KnownUser>();
    for (const u of extra.data ?? EMPTY) map.set(u.id, u);
    for (const u of toKnown(dir.data ?? [])) map.set(u.id, u);
    return [...map.values()].sort((a, b) => a.name.localeCompare(b.name));
  }, [dir.data, extra.data]);
}

/** Personen mit Zugriff auf das Team (`users(team:)`): Auswahl für Assignee, nie Nutzer ohne Teamzugang. */
export function useAssignableUsers(teamId: string | undefined): KnownUser[] {
  const dir = useDirectoryUsers("", teamId, !!teamId);
  return useMemo(
    () => toKnown(dir.data ?? []).sort((a, b) => a.name.localeCompare(b.name)),
    [dir.data],
  );
}

/** @deprecated Kein Modulzustand mehr; bleibt als No-op, damit bestehende Tests kompilieren. */
export function resetKnownUsers(): void {}

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
