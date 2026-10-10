import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { cn } from "@/lib/cn";

import { useWorkload } from "./queries";

/** Auslastung: Person × Team, offene Issues und Punkte im aktuellen/nächsten Cycle (docs/14 §2). */
export function WorkloadPage() {
  const [cycle, setCycle] = useState<"current" | "next">("current");
  const q = useWorkload(cycle);
  useEffect(() => {
    document.title = "Workload · Lineal";
  }, []);
  const teams = useMemo(() => {
    const m = new Map<string, string>();
    for (const r of q.data ?? []) for (const t of r.perTeam) m.set(t.teamKey, t.teamName);
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [q.data]);

  let body;
  if (q.isLoading) body = <ListSkeleton rows={6} />;
  else if (q.isError)
    body = (
      <ErrorState
        error={q.error}
        title="Could not load workload"
        onRetry={() => void q.refetch()}
      />
    );
  else if (!q.data || q.data.length === 0)
    body = <EmptyState title="No workload" description="No open issues in this cycle." />;
  else
    body = (
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs text-muted-foreground">
            <th className="px-4 py-2 font-medium">Person</th>
            {teams.map(([key, name]) => (
              <th key={key} className="px-3 py-2 font-medium" title={name}>
                {key}
              </th>
            ))}
            <th className="px-3 py-2 font-medium">Total</th>
          </tr>
        </thead>
        <tbody>
          {q.data.map((r) => (
            <tr key={r.user.id} className="border-b border-border">
              <td className="px-4 py-1.5">{r.user.displayName || r.user.name}</td>
              {teams.map(([key]) => {
                const e = r.perTeam.find((t) => t.teamKey === key);
                return (
                  <td key={key} className="px-3 py-1.5 tabular-nums">
                    {e ? `${e.openIssues} · ${e.points} pt` : "–"}
                  </td>
                );
              })}
              <td
                className={cn(
                  "px-3 py-1.5 font-medium tabular-nums",
                  r.perTeam.length > 1 && "text-primary",
                )}
              >
                {r.totalOpenIssues} · {r.totalPoints} pt
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
        <h1 className="text-sm font-medium">Workload</h1>
        <div role="group" aria-label="Cycle" className="ml-auto flex gap-1">
          {(["current", "next"] as const).map((c) => (
            <Button
              key={c}
              variant="ghost"
              size="sm"
              aria-pressed={cycle === c}
              className={cn(cycle === c && "bg-surface-active")}
              onClick={() => setCycle(c)}
            >
              {c === "current" ? "Current cycle" : "Next cycle"}
            </Button>
          ))}
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
