import { Link } from "@tanstack/react-router";
import { useEffect } from "react";

import { teamPaths } from "@/app/nav";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";

import { useActiveCycles } from "./queries";

const pct = (n: number) => `${Math.round(n * 100)}%`;

/** Cycle-Übersicht (docs/14 §2): aktive Cycles aller Teams nebeneinander. */
export function CyclesOverviewPage({ ws }: { ws: string }) {
  const q = useActiveCycles();
  useEffect(() => {
    document.title = "Cycles · Lineal";
  }, []);

  let body;
  if (q.isLoading) body = <ListSkeleton rows={4} />;
  else if (q.isError)
    body = (
      <ErrorState error={q.error} title="Could not load cycles" onRetry={() => void q.refetch()} />
    );
  else if (!q.data || q.data.length === 0)
    body = (
      <EmptyState title="No active cycles" description="Teams with cycles enabled show up here." />
    );
  else
    body = (
      <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-3">
        {q.data.map((c) => (
          <article
            key={c.cycleId}
            aria-label={`Cycle ${c.teamKey}`}
            className="flex flex-col gap-2 rounded-lg border border-border bg-surface p-3"
          >
            <header className="flex items-center gap-2">
              <Link
                to={teamPaths(ws, c.teamKey).currentCycle as "/"}
                className="text-sm font-medium hover:underline"
              >
                {c.teamName}
              </Link>
              <span className="text-xs text-muted-foreground">{c.name ?? `Cycle ${c.number}`}</span>
              <span className="ml-auto text-xs text-muted-foreground">
                {c.remainingDays} days left
              </span>
            </header>
            <div
              role="progressbar"
              aria-valuenow={Math.round(c.progress * 100)}
              aria-valuemin={0}
              aria-valuemax={100}
              className="h-1.5 overflow-hidden rounded-full bg-surface-active"
            >
              <div className="h-full bg-primary" style={{ width: pct(c.progress) }} />
            </div>
            <p className="text-xs text-muted-foreground tabular-nums">
              {pct(c.progress)} complete · scope {c.scope} · scope change{" "}
              {c.scopeChange > 0 ? `+${c.scopeChange}` : c.scopeChange}
            </p>
          </article>
        ))}
      </div>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center border-b border-border px-3">
        <h1 className="text-sm font-medium">Cycles</h1>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
