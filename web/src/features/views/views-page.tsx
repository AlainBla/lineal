import { Link } from "@tanstack/react-router";
import { Plus, Star } from "lucide-react";
import { useState } from "react";

import { useActions } from "@/app/commands";
import { Button } from "@/components/ui/button";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { CustomView } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { useCustomView, useCustomViews, useFavorites, useToggleFavorite } from "./queries";
import { IssueView } from "./issue-view";
import { ViewBuilderDialog } from "./view-builder";

/** Stern-Schalter „Add to favorites“ für Views. */
function FavoriteStar({ view }: { view: CustomView }) {
  const favs = useFavorites();
  const { add, remove } = useToggleFavorite();
  const fav = favs.data?.find((f) => f.type === "customView" && f.targetId === view.id);
  return (
    <Button
      variant="ghost"
      size="icon"
      aria-label={fav ? "Remove from favorites" : "Add to favorites"}
      aria-pressed={!!fav}
      onClick={() =>
        fav ? remove.mutate(fav.id) : add.mutate({ type: "customView", targetId: view.id })
      }
    >
      <Star className={fav ? "fill-current text-amber-400" : undefined} />
    </Button>
  );
}

/** Views-Liste (`/views/issues`, `/team/KEY/views/issues`): gespeicherte Views, „New view“ (Alt V). */
export function ViewsPage({
  ws,
  teamId,
  teamKey,
}: {
  ws: string;
  teamId?: string;
  teamKey?: string;
}) {
  const q = useCustomViews({ teamId });
  const [building, setBuilding] = useState(false);
  useShortcutContext("list");
  useActions([
    {
      id: "view.create",
      title: "Create view…",
      section: "Views",
      keys: "alt+v",
      perform: () => setBuilding(true),
    },
  ]);

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState error={q.error} title="Could not load views" onRetry={() => void q.refetch()} />
    );
  else if (!q.data?.length)
    body = (
      <EmptyState
        title="No views yet"
        description="Save a set of filters as a view to come back to it."
        action={
          <Button size="sm" onClick={() => setBuilding(true)}>
            <Plus /> New view
          </Button>
        }
      />
    );
  else
    body = (
      <ul className="divide-y divide-border">
        {q.data.map((v) => (
          <li key={v.id} className="flex items-center gap-3 px-4 py-2 text-sm">
            <Link to={`/${ws}/view/${v.slugId}` as "/"} className="font-medium hover:underline">
              {v.name}
            </Link>
            <span className="text-xs text-muted-foreground">{v.modelName}</span>
            <span className="text-xs text-muted-foreground">
              {v.teamKey ?? (v.shared ? "Workspace" : "Personal")}
            </span>
            {v.description && (
              <span className="truncate text-xs text-subtle-foreground">{v.description}</span>
            )}
            <span className="ml-auto">
              <FavoriteStar view={v} />
            </span>
          </li>
        ))}
      </ul>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
        <h1 className="text-sm font-medium">{teamKey ? `${teamKey} · Views` : "Views"}</h1>
        <Button size="sm" className="ml-auto" onClick={() => setBuilding(true)}>
          <Plus /> New view
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
      {building && <ViewBuilderDialog ws={ws} teamId={teamId} onClose={() => setBuilding(false)} />}
    </div>
  );
}

/** Eine gespeicherte View (`/view/{ref}`). */
export function ViewPage({ ws, viewRef }: { ws: string; viewRef: string }) {
  const q = useCustomView(viewRef);
  if (q.isLoading) return <ListSkeleton />;
  if (q.isError || !q.data)
    return (
      <ErrorState error={q.error} title="Could not load view" onRetry={() => void q.refetch()} />
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">
        <IssueView key={q.data.id} ws={ws} view={q.data} title={q.data.name} />
      </div>
    </div>
  );
}
