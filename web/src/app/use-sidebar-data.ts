/**
 * Sidebar-Daten (Teams, Sub-Teams, Badges) hinter einer stabilen Schnittstelle; der Sync-Store
 * (WP15) kann die Implementierung von `useSidebarData` ersetzen, die Form `SidebarData` bleibt.
 */
import { useQueries, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { useSession } from "@/app/auth";
import { fetchTriageCount, triageKeys } from "@/features/triage/queries";
import { gqlRequest } from "@/lib/graphql";

export interface SidebarTeam {
  id: string;
  /** Team-Key, z. B. `ENG`. */
  key: string;
  name: string;
  /** Parent-Team (Sub-Team) oder `null`. */
  parentId: string | null;
  cyclesEnabled: boolean;
  triageEnabled: boolean;
  /** Zähler nicht gesnoozter Triage-Issues. */
  triageCount: number;
}

export interface SidebarData {
  /** Teams, in denen der User Mitglied ist, nach `sortOrder` sortiert. */
  teams: SidebarTeam[];
  inboxUnread: number;
  draftsCount: number;
}

export interface SidebarQueryResult {
  data: SidebarData | undefined;
  isLoading: boolean;
  isError: boolean;
}

/** Erfundene Beispieldaten (acme: ENG, WEB als Sub-Team von ENG, DES). */
export const MOCK_SIDEBAR_DATA: SidebarData = {
  teams: [
    {
      id: "team-eng",
      key: "ENG",
      name: "Engineering",
      parentId: null,
      cyclesEnabled: true,
      triageEnabled: true,
      triageCount: 2,
    },
    {
      id: "team-web",
      key: "WEB",
      name: "Web",
      parentId: "team-eng",
      cyclesEnabled: true,
      triageEnabled: false,
      triageCount: 0,
    },
    {
      id: "team-des",
      key: "DES",
      name: "Design",
      parentId: null,
      cyclesEnabled: false,
      triageEnabled: false,
      triageCount: 0,
    },
  ],
  inboxUnread: 3,
  draftsCount: 1,
};

export interface TeamNode {
  team: SidebarTeam;
  children: TeamNode[];
}

/**
 * Baut den Teambaum. Sub-Teams stehen eingerückt unter ihrem Parent; fehlt der Parent in der Liste
 * (z. B. Gast nur im Sub-Team), erscheint das Sub-Team als Wurzel. Die Reihenfolge der Eingabe bleibt erhalten.
 */
export function buildTeamTree(teams: SidebarTeam[]): TeamNode[] {
  const nodes = new Map<string, TeamNode>(teams.map((t) => [t.id, { team: t, children: [] }]));
  const roots: TeamNode[] = [];
  for (const t of teams) {
    const node = nodes.get(t.id)!;
    const parent = t.parentId ? nodes.get(t.parentId) : undefined;
    if (parent && parent !== node) parent.children.push(node);
    else roots.push(node);
  }
  return roots;
}

/** Flache Reihenfolge wie in der Sidebar (Parent, dann seine Sub-Teams) – Grundlage für `Ctrl ⇧ 1…9`. */
export function flattenTeamTree(nodes: TeamNode[]): SidebarTeam[] {
  return nodes.flatMap((n) => [n.team, ...flattenTeamTree(n.children)]);
}

const UNREAD_QUERY = "query SidebarUnread { notificationsUnreadCount }";

/**
 * Teams kommen aus der Session (`/auth/me`), der Inbox-Zähler aus GraphQL. Triage- und
 * Draft-Zähler folgen mit WP12 bzw. dem Sync-Store (WP15).
 */
export function useSidebarData(): SidebarQueryResult {
  const session = useSession();
  const q = useQuery({
    queryKey: ["sidebar-data", session?.user.id],
    enabled: !!session,
    queryFn: async (): Promise<SidebarData> => {
      let inboxUnread = 0;
      try {
        const res = await gqlRequest<{ notificationsUnreadCount: number }>(UNREAD_QUERY);
        inboxUnread = res.notificationsUnreadCount;
      } catch {
        // Zähler ist nicht kritisch; Sidebar bleibt nutzbar.
      }
      return {
        teams: (session?.teams ?? []).map((t) => ({ ...t, triageCount: 0 })),
        inboxUnread,
        draftsCount: 0,
      };
    },
    staleTime: 30_000,
  });
  // Triage-Zähler je Team mit Triage; Key unter `["issues"]`, damit jede Issue-Änderung ihn erneuert.
  const triageTeams = (session?.teams ?? []).filter((t) => t.triageEnabled);
  const counts = useQueries({
    queries: triageTeams.map((t) => ({
      queryKey: triageKeys.count(t.id),
      queryFn: ({ signal }: { signal: AbortSignal }) => fetchTriageCount(t.id, signal),
      staleTime: 30_000,
    })),
    combine: (results) => results.map((r) => r.data ?? 0),
  });
  const countKey = counts.join(",");
  const data = useMemo(() => {
    if (!q.data) return undefined;
    const byId = new Map(triageTeams.map((t, i) => [t.id, counts[i] ?? 0]));
    return {
      ...q.data,
      teams: q.data.teams.map((t) => ({ ...t, triageCount: byId.get(t.id) ?? t.triageCount })),
    };
    // `counts` ist über `countKey` stabilisiert.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q.data, countKey]);
  return { data, isLoading: q.isLoading, isError: q.isError };
}
