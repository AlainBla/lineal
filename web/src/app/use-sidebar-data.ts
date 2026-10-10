/**
 * Sidebar-Daten (Teams, Sub-Teams, Badges). Vorerst Mock-Daten hinter einer stabilen Schnittstelle:
 * später ersetzt eine GraphQL-Abfrage (bzw. der Sync-Store) die Implementierung von
 * `useSidebarData`, die Form `SidebarData` bleibt gleich.
 */
import { useQuery } from "@tanstack/react-query";

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

export function useSidebarData(): SidebarQueryResult {
  const q = useQuery({
    queryKey: ["sidebar-data"],
    // TODO(WP13/14): durch GraphQL-Abfrage bzw. Sync-Store ersetzen.
    queryFn: async (): Promise<SidebarData> => MOCK_SIDEBAR_DATA,
    staleTime: Infinity,
  });
  return { data: q.data, isLoading: q.isLoading, isError: q.isError };
}
