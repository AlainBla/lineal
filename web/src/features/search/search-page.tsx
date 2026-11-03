import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Search as SearchIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { Input } from "@/components/ui/input";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { EntitySearchConnection, EntitySearchResult } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { gqlRequest } from "@/lib/graphql";

export type SearchTab = "all" | "Issue" | "Project" | "Initiative" | "Document";
const TABS: Array<[SearchTab, string]> = [
  ["all", "All"],
  ["Issue", "Issues"],
  ["Project", "Projects"],
  ["Initiative", "Initiatives"],
  ["Document", "Documents"],
];

export const searchKeys = {
  results: (term: string, tab: SearchTab) => ["search", term, tab] as const,
};

export function useSearch(term: string, tab: SearchTab) {
  return useQuery({
    queryKey: searchKeys.results(term, tab),
    enabled: term.trim().length > 0,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ search: EntitySearchConnection }, Record<string, unknown>>(
          `query GlobalSearch($term: String!, $types: [String!], $first: Int) {
            search(term: $term, types: $types, first: $first) {
              nodes { id kind identifier title teamKey status archived updatedAt highlights { field snippet } }
              hasNextPage endCursor
            }
          }`,
          { term: term.trim(), types: tab === "all" ? undefined : [tab], first: 50 },
          { signal },
        )
      ).search.nodes,
  });
}

/** Ziel eines Treffers (docs/08 §1.3). */
export function resultPath(
  ws: string,
  r: Pick<EntitySearchResult, "kind" | "id" | "identifier">,
): string {
  const ref = r.identifier ?? r.id;
  switch (r.kind) {
    case "Issue":
      return `/${ws}/issue/${ref}`;
    case "Project":
      return `/${ws}/project/${ref}/overview`;
    case "Initiative":
      return `/${ws}/initiative/${ref}/overview`;
    default:
      return `/${ws}/document/${r.id}`;
  }
}

/** Suche (`/{ws}/search?q=`, docs/05 §5, F14): Tabs nach Typ, Treffer mit Kennung, Team und Status. */
export function SearchPage({
  ws,
  initialTerm,
  onTermChange,
}: {
  ws: string;
  initialTerm: string;
  onTermChange?: (term: string) => void;
}) {
  const [input, setInput] = useState(initialTerm);
  const [term, setTerm] = useState(initialTerm);
  const [tab, setTab] = useState<SearchTab>("all");
  useEffect(() => {
    const t = setTimeout(() => {
      setTerm(input);
      onTermChange?.(input);
    }, 250);
    return () => clearTimeout(t);
  }, [input, onTermChange]);
  useEffect(() => {
    document.title = "Search · Lineal";
  }, []);
  const q = useSearch(term, tab);

  let body;
  if (term.trim() === "")
    body = (
      <EmptyState
        title="Search Lineal"
        description="Find issues, projects, initiatives and documents."
      />
    );
  else if (q.isLoading) body = <ListSkeleton rows={5} />;
  else if (q.isError)
    body = <ErrorState error={q.error} title="Search failed" onRetry={() => void q.refetch()} />;
  else if (!q.data?.length) body = <EmptyState title={`No results for “${term}”`} />;
  else
    body = (
      <ul aria-label="Results" className="divide-y divide-border">
        {q.data.map((r) => (
          <li key={`${r.kind}-${r.id}`}>
            <Link
              to={resultPath(ws, r) as "/"}
              className="flex items-center gap-3 px-4 py-2 text-sm hover:bg-surface-hover"
            >
              <span className="w-16 shrink-0 text-xs text-muted-foreground">{r.kind}</span>
              {r.identifier && (
                <span className="text-xs text-muted-foreground">{r.identifier}</span>
              )}
              <span className="truncate">{r.title}</span>
              {r.archived && <span className="text-xs text-subtle-foreground">archived</span>}
              <span className="ml-auto text-xs text-muted-foreground">
                {[r.teamKey, r.status].filter(Boolean).join(" · ")}
              </span>
            </Link>
          </li>
        ))}
      </ul>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-col gap-2 border-b border-border px-3 py-2">
        <div className="relative">
          <SearchIcon
            className="absolute top-2 left-2.5 size-4 text-subtle-foreground"
            aria-hidden
          />
          <Input
            aria-label="Search"
            placeholder="Search issues, projects, documents…"
            className="pl-8"
            autoFocus
            value={input}
            onChange={(e) => setInput(e.target.value)}
          />
        </div>
        <nav aria-label="Result types" className="flex gap-1">
          {TABS.map(([t, label]) => (
            <button
              key={t}
              type="button"
              aria-pressed={tab === t}
              onClick={() => setTab(t)}
              className={cn(
                "rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                tab === t && "bg-surface-active text-foreground",
              )}
            >
              {label}
            </button>
          ))}
        </nav>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
