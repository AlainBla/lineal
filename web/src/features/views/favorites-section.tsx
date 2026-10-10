import { Link } from "@tanstack/react-router";
import { Folder, Star, X } from "lucide-react";
import { useMemo } from "react";

import { useSession } from "@/app/auth";
import { teamPaths } from "@/app/nav";
import type { Favorite } from "@/gql/graphql";

import { useCustomViews, useFavorites, useToggleFavorite } from "./queries";

const PREDEFINED: Record<string, { label: string; path: (ws: string, key: string) => string }> = {
  activeIssues: { label: "Active", path: (ws, k) => teamPaths(ws, k).active },
  backlog: { label: "Backlog", path: (ws, k) => teamPaths(ws, k).backlog },
  allIssues: { label: "All issues", path: (ws, k) => teamPaths(ws, k).all },
  triage: { label: "Triage", path: (ws, k) => teamPaths(ws, k).triage },
  activeCycle: { label: "Current cycle", path: (ws, k) => teamPaths(ws, k).currentCycle },
  upcomingCycle: { label: "Upcoming cycle", path: (ws, k) => teamPaths(ws, k).upcomingCycle },
};

export interface FavoriteTarget {
  label: string;
  to: string;
}

/** Ziel und Anzeigename eines Favoriten; `null`, wenn der Name (noch) nicht auflösbar ist. */
export function resolveFavorite(
  f: Favorite,
  ws: string,
  ctx: { views: Map<string, { name: string; slugId: string }>; teams: Map<string, string> },
): FavoriteTarget | null {
  switch (f.type) {
    case "customView": {
      const v = f.targetId ? ctx.views.get(f.targetId) : undefined;
      return v ? { label: v.name, to: `/${ws}/view/${v.slugId}` } : null;
    }
    case "predefinedView": {
      const p = f.predefinedViewType ? PREDEFINED[f.predefinedViewType] : undefined;
      const key = f.predefinedViewTeamId ? ctx.teams.get(f.predefinedViewTeamId) : undefined;
      return p && key ? { label: `${p.label} · ${key}`, to: p.path(ws, key) } : null;
    }
    case "issue":
      return f.targetId ? { label: "Issue", to: `/${ws}/issue/${f.targetId}` } : null;
    case "project":
      return f.targetId ? { label: "Project", to: `/${ws}/project/${f.targetId}/overview` } : null;
    case "initiative":
      return f.targetId
        ? { label: "Initiative", to: `/${ws}/initiative/${f.targetId}/overview` }
        : null;
    case "document":
      return f.targetId ? { label: "Document", to: `/${ws}/document/${f.targetId}` } : null;
    case "team": {
      const key = f.targetId ? ctx.teams.get(f.targetId) : undefined;
      return key ? { label: key, to: teamPaths(ws, key).all } : null;
    }
    default:
      return null;
  }
}

/**
 * Favoriten-Bereich der Sidebar (docs/05 §4.6, über „Your teams“). Wird von `sidebar.tsx` eingehängt;
 * ohne Favoriten rendert er nichts.
 */
export function FavoritesSection({ ws }: { ws: string }) {
  const favs = useFavorites();
  const views = useCustomViews();
  const session = useSession();
  const { remove } = useToggleFavorite();
  const ctx = useMemo(
    () => ({
      views: new Map((views.data ?? []).map((v) => [v.id, { name: v.name, slugId: v.slugId }])),
      teams: new Map((session?.teams ?? []).map((t) => [t.id, t.key])),
    }),
    [views.data, session?.teams],
  );
  const items = favs.data ?? [];
  if (items.length === 0) return null;

  const top = items.filter((f) => !f.parentId).sort((a, b) => a.sortOrder - b.sortOrder);
  const row = (f: Favorite) => {
    const t = resolveFavorite(f, ws, ctx);
    if (!t) return null;
    return (
      <li key={f.id} className="group flex items-center">
        <Link
          to={t.to as "/"}
          className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground hover:bg-surface-hover"
        >
          <Star className="size-3.5 shrink-0 text-subtle-foreground" aria-hidden />
          <span className="truncate">{t.label}</span>
        </Link>
        <button
          type="button"
          aria-label={`Remove favorite ${t.label}`}
          className="mr-1 hidden size-5 items-center justify-center rounded text-subtle-foreground group-hover:flex hover:text-foreground"
          onClick={() => remove.mutate(f.id)}
        >
          <X className="size-3" />
        </button>
      </li>
    );
  };

  return (
    <section aria-label="Favorites">
      <h2 className="px-2 pt-4 pb-1 text-xs font-medium text-subtle-foreground select-none">
        Favorites
      </h2>
      <ul className="flex flex-col gap-px">
        {top.map((f) =>
          f.type === "folder" ? (
            <li key={f.id}>
              <div className="flex h-7 items-center gap-2 px-2 text-sm font-medium text-sidebar-foreground">
                <Folder className="size-3.5 text-subtle-foreground" aria-hidden />
                {f.folderName}
              </div>
              <ul className="ml-3 flex flex-col gap-px border-l border-border pl-1">
                {items
                  .filter((c) => c.parentId === f.id)
                  .sort((a, b) => a.sortOrder - b.sortOrder)
                  .map(row)}
              </ul>
            </li>
          ) : (
            row(f)
          ),
        )}
      </ul>
    </section>
  );
}
