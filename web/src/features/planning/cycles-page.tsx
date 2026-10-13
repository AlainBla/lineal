import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { teamPaths } from "@/app/nav";
import { DocumentsSection } from "@/features/docs/documents-section";
import { setCreateContext } from "@/features/issues/create-store";
import { useTeamMeta } from "@/features/issues/hooks";
import { IssueList } from "@/features/issues/issue-list";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { groupByState } from "@/features/issues/model";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Cycle } from "@/gql/graphql";

import { useCycle, useScopedIssues, useTeamCycles, useTeamTimezone } from "./queries";
import { cycleRange, PageHeader, ProgressBar } from "./ui";

export function cycleBadge(c: Cycle): string {
  if (c.isActive) return "Current";
  if (c.isNext || c.isFuture) return c.isNext ? "Upcoming" : "Planned";
  return "Completed";
}

/** Kapazitätsschätzung (Punkte), ohne überflüssige Nachkommastellen. */
export const formatCapacity = (n: number): string =>
  Number.isInteger(n) ? String(n) : n.toFixed(1);

const cycleName = (c: Cycle) => c.name ?? `Cycle ${c.number}`;

/** Cycles eines Teams (docs/08 §2.6): Timeline-Liste, neueste zuerst. */
export function TeamCyclesPage({ ws, teamKey }: { ws: string; teamKey: string }) {
  const q = useTeamCycles(teamKey);
  const tz = useTeamTimezone(teamKey);
  const p = teamPaths(ws, teamKey);
  let body;
  if (q.isLoading) body = <ListSkeleton rows={5} />;
  else if (q.isError)
    body = (
      <ErrorState error={q.error} title="Could not load cycles" onRetry={() => void q.refetch()} />
    );
  else if (!q.data || q.data.length === 0)
    body = <EmptyState title="No cycles" description="Enable cycles in the team settings." />;
  else
    body = (
      <ul className="divide-y divide-border">
        {[...q.data]
          .sort((a, b) => b.number - a.number)
          .map((c) => (
            <li key={c.id} className="flex items-center gap-3 px-4 py-2 text-sm">
              <Link
                to={`/${ws}/team/${teamKey}/cycle/${c.id}` as "/"}
                className="font-medium hover:underline"
              >
                {cycleName(c)}
              </Link>
              <span className="rounded-full bg-surface-active px-1.5 text-xs text-muted-foreground">
                {cycleBadge(c)}
              </span>
              <span className="text-xs text-muted-foreground">
                {cycleRange(c.startsAt, c.endsAt, tz)}
              </span>
              <span className="ml-auto flex items-center gap-3">
                <span className="text-xs text-muted-foreground tabular-nums">
                  {c.completedIssueCount} / {c.issueCount} issues
                </span>
                <ProgressBar value={c.progress} label={`Progress of ${cycleName(c)}`} />
              </span>
            </li>
          ))}
      </ul>
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={`${teamKey} · Cycles`}
        docTitle={`${teamKey} · Cycles`}
        tabs={[
          { label: "All", to: p.cycles, active: true },
          { label: "Current", to: p.currentCycle, active: false },
          { label: "Upcoming", to: p.upcomingCycle, active: false },
        ]}
      />
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}

/** Cycle-Detail: Issue-Liste und Fortschritt (`ref` = UUID, `active` oder `upcoming`). */
export function CyclePage({
  ws,
  teamKey,
  cycleRef,
}: {
  ws: string;
  teamKey: string;
  cycleRef: string;
}) {
  const cq = useCycle(teamKey, cycleRef);
  const tz = useTeamTimezone(teamKey);
  const cycle = cq.data ?? null;
  const issues = useScopedIssues(cycle ? { cycleId: cycle.id } : null);
  const meta = useTeamMeta(teamKey);
  const lookup = useIssueLookup([meta.data]);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const p = teamPaths(ws, teamKey);
  // „C“ legt im Cycle-Kontext an (B4.15).
  const ctxCycleId = cycle?.id;
  useEffect(() => {
    if (!ctxCycleId) return;
    setCreateContext({ teamKey, cycleId: ctxCycleId });
    return () => setCreateContext(null);
  }, [ctxCycleId, teamKey]);

  if (cq.isLoading) return <ListSkeleton />;
  if (cq.isError)
    return (
      <ErrorState error={cq.error} title="Could not load cycle" onRetry={() => void cq.refetch()} />
    );
  if (!cycle)
    return (
      <EmptyState
        title={
          cycleRef === "upcoming"
            ? "No upcoming cycle"
            : cycleRef === "active"
              ? "No active cycle"
              : "Cycle not found"
        }
        description={
          cycleRef === "active" || cycleRef === "upcoming"
            ? "Cycles appear here once they are scheduled."
            : "This cycle does not exist in this team."
        }
      />
    );
  const groups = meta.data ? groupByState(meta.data.states, issues.data?.issues ?? [], "all") : [];
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={`${teamKey} · ${cycleName(cycle)}`}
        docTitle={`${teamKey} · ${cycleName(cycle)}`}
        tabs={[
          { label: "All cycles", to: p.cycles, active: false },
          { label: "Current", to: p.currentCycle, active: cycleRef === "active" },
          { label: "Upcoming", to: p.upcomingCycle, active: cycleRef === "upcoming" },
        ]}
        actions={
          <span className="text-xs text-muted-foreground">
            {cycleRange(cycle.startsAt, cycle.endsAt, tz)} · {Math.round(cycle.progress * 100)}%
            {cycle.capacity != null && (
              <>
                {" "}
                · Capacity{" "}
                <span data-testid="cycle-capacity">{formatCapacity(cycle.capacity)}</span>
              </>
            )}
          </span>
        }
      />
      <div className="min-h-0 flex-1 overflow-auto">
        {issues.isLoading ? (
          <ListSkeleton />
        ) : groups.length === 0 ? (
          <EmptyState title="No issues in this cycle" />
        ) : (
          <IssueList
            groups={groups}
            ws={ws}
            lookup={lookup}
            focusedId={focusedId}
            onFocus={setFocusedId}
          />
        )}
        <DocumentsSection ws={ws} scope={{ cycleId: cycle.id }} teamId={meta.data?.team.id} />
      </div>
    </div>
  );
}
