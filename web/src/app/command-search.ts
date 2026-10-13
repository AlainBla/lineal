/**
 * Objektsuche des Command-Menüs (F.15, docs/05 §5.6): Präfixe `i ` Issues, `p ` Projekte, `u ` User,
 * `t ` Teams, `l ` Labels, `f ` Favoriten, `d ` Dokumente (zusätzlich `n ` Initiativen). Ohne Präfix
 * erscheinen unter den Befehlen bis zu 5 „Quick results“ aus der Issue-Schnellsuche.
 */
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { useSession } from "@/app/auth";
import { teamPaths } from "@/app/nav";
import { resolveFavorite } from "@/features/views/favorites-section";
import { useCustomViews, useFavorites } from "@/features/views/queries";
import { resultPath } from "@/features/search/search-page";
import type { EntitySearchResult, IssueLabel, UserRef } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export type SearchPrefix = "i" | "p" | "u" | "t" | "l" | "f" | "d" | "n";

export const PREFIXES: Record<SearchPrefix, { label: string; placeholder: string }> = {
  i: { label: "Issues", placeholder: "Search issues…" },
  p: { label: "Projects", placeholder: "Search projects…" },
  u: { label: "Users", placeholder: "Search users…" },
  t: { label: "Teams", placeholder: "Search teams…" },
  l: { label: "Labels", placeholder: "Search labels…" },
  f: { label: "Favorites", placeholder: "Search favorites…" },
  d: { label: "Documents", placeholder: "Search documents…" },
  n: { label: "Initiatives", placeholder: "Search initiatives…" },
};

export interface ParsedQuery {
  prefix: SearchPrefix | null;
  term: string;
}

/** `i login` → Präfix `i`, Begriff `login`; ohne Präfix bleibt der ganze Text der Begriff. */
export function parseCommandQuery(input: string): ParsedQuery {
  const m = /^([ipultfdn])\s(.*)$/i.exec(input);
  if (m) return { prefix: m[1]!.toLowerCase() as SearchPrefix, term: m[2]!.trim() };
  return { prefix: null, term: input.trim() };
}

export interface ObjectHit {
  /** Eindeutig über alle Arten. */
  id: string;
  kind: string;
  title: string;
  hint?: string;
  /** Interner Zielpfad. */
  to: string;
}

/** Quick results ohne Präfix: höchstens so viele. */
export const QUICK_RESULTS = 5;

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

async function searchEntities(term: string, types: string[], first: number, signal?: AbortSignal) {
  const res = await gqlRequest<
    { search: { nodes: EntitySearchResult[] } },
    { term: string; types: string[]; first: number }
  >(
    `query CommandSearch($term: String!, $types: [String!], $first: Int) {
      search(term: $term, types: $types, first: $first) {
        nodes { id kind identifier title teamKey status archived updatedAt highlights { field snippet } }
      }
    }`,
    { term, types, first },
    { signal },
  );
  return res.search.nodes;
}

const TYPE_FOR: Partial<Record<SearchPrefix, string>> = {
  i: "Issue",
  p: "Project",
  d: "Document",
  n: "Initiative",
};

/**
 * Treffer zu Präfix und Begriff. Server-Suchen laufen entprellt (150 ms) und nur bei nicht-leerem
 * Begriff (Teams und Favoriten filtern lokal und zeigen ohne Begriff alle).
 */
export function useObjectHits(parsed: ParsedQuery): { hits: ObjectHit[]; loading: boolean } {
  const session = useSession();
  const ws = session?.workspace.urlKey ?? "";
  const term = useDebounced(parsed.term, 150);
  const stale = term !== parsed.term;
  const { prefix } = parsed;
  const quick = prefix === null && term.length >= 2;
  const needsServer = prefix === "i" || prefix === "p" || prefix === "d" || prefix === "n" || quick;
  const entityType = prefix ? TYPE_FOR[prefix] : "Issue";

  const entities = useQuery({
    queryKey: ["command-search", entityType, term, quick],
    enabled: needsServer && term.length > 0 && !!entityType,
    queryFn: ({ signal }) =>
      searchEntities(term, [entityType!], quick ? QUICK_RESULTS : 10, signal),
    staleTime: 15_000,
  });
  const users = useQuery({
    queryKey: ["command-search", "users", term],
    enabled: prefix === "u",
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ users: UserRef[] }, { query: string | null; first: number }>(
          `query CommandUsers($query: String, $first: Int!) { users(query: $query, first: $first) { id name displayName } }`,
          { query: term || null, first: 10 },
          { signal },
        )
      ).users,
    staleTime: 15_000,
  });
  const labels = useQuery({
    queryKey: ["command-search", "labels", term],
    enabled: prefix === "l",
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ issueLabels: IssueLabel[] }, { name: string | null; first: number }>(
          `query CommandLabels($name: String, $first: Int) { issueLabels(name: $name, first: $first) { id name teamId isGroup } }`,
          { name: term || null, first: 10 },
          { signal },
        )
      ).issueLabels,
    staleTime: 15_000,
  });
  const favorites = useFavorites();
  const views = useCustomViews();

  const lower = parsed.term.toLowerCase();
  let hits: ObjectHit[] = [];
  let loading = stale && needsServer;
  if (needsServer) {
    loading = loading || entities.isFetching;
    hits = (entities.data ?? []).map((r) => ({
      id: `${r.kind}:${r.id}`,
      kind: r.kind,
      title: r.identifier ? `${r.identifier} ${r.title}` : r.title,
      hint: r.status ?? undefined,
      to: resultPath(ws, r),
    }));
  } else if (prefix === "u") {
    loading = users.isFetching;
    hits = (users.data ?? []).map((u) => ({
      id: `user:${u.id}`,
      kind: "User",
      title: u.name,
      hint: u.displayName !== u.name ? u.displayName : undefined,
      to: `/${ws}/search?q=${encodeURIComponent(u.name)}`,
    }));
  } else if (prefix === "t") {
    hits = (session?.teams ?? [])
      .filter((t) => !lower || `${t.name} ${t.key}`.toLowerCase().includes(lower))
      .slice(0, 10)
      .map((t) => ({
        id: `team:${t.id}`,
        kind: "Team",
        title: t.name,
        hint: t.key,
        to: teamPaths(ws, t.key).home,
      }));
  } else if (prefix === "l") {
    loading = labels.isFetching;
    hits = (labels.data ?? [])
      .filter((l) => !l.isGroup)
      .map((l) => ({
        id: `label:${l.id}`,
        kind: "Label",
        title: l.name,
        to: `/${ws}/settings/labels`,
      }));
  } else if (prefix === "f") {
    const teamMap = new Map((session?.teams ?? []).map((t) => [t.id, t.key]));
    const viewMap = new Map((views.data ?? []).map((v) => [v.id, v]));
    for (const f of favorites.data ?? []) {
      const t = resolveFavorite(f, ws, { views: viewMap, teams: teamMap });
      if (!t || (lower && !t.label.toLowerCase().includes(lower))) continue;
      hits.push({ id: `favorite:${f.id}`, kind: "Favorite", title: t.label, to: t.to });
    }
    hits = hits.slice(0, 10);
  }
  return { hits, loading };
}
