import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { teamPaths, workspacePaths } from "@/app/nav";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { OverviewCounts, ProjectHealthSummary, TeamMetrics } from "@/gql/graphql";
import { cn } from "@/lib/cn";

import { useOverview } from "./queries";

function Stat({ label, value, warn }: { label: string; value: number | string; warn?: boolean }) {
  return (
    <div className="flex flex-col">
      <dd className={cn("text-base font-semibold tabular-nums", warn && "text-destructive")}>
        {value}
      </dd>
      <dt className="text-xs text-muted-foreground">{label}</dt>
    </div>
  );
}

function Health({ h }: { h: ProjectHealthSummary }) {
  return (
    <p className="text-xs text-muted-foreground" aria-label="Project health">
      Projects: <span className="text-emerald-500">{h.onTrack} on track</span> ·{" "}
      <span className="text-amber-500">{h.atRisk} at risk</span> ·{" "}
      <span className="text-red-500">{h.offTrack} off track</span> · {h.noUpdate} no update
    </p>
  );
}

function Counts({ c }: { c: OverviewCounts }) {
  return (
    <dl className="grid grid-cols-4 gap-2">
      <Stat label="Open" value={c.openIssues} />
      <Stat label="In progress" value={c.inProgress} />
      <Stat label="Triage" value={c.triage} />
      <Stat label="Done" value={c.completed} />
      <Stat label="Overdue" value={c.overdue} warn={c.overdue > 0} />
      <Stat label="SLA risk" value={c.slaAtRisk} warn={c.slaAtRisk > 0} />
      <Stat label="SLA breached" value={c.slaBreached} warn={c.slaBreached > 0} />
    </dl>
  );
}

function TeamTile({ t, ws }: { t: TeamMetrics; ws: string }) {
  return (
    <article
      aria-label={`Team ${t.key}`}
      data-team-key={t.key}
      className="flex flex-col gap-3 rounded-lg border border-border bg-surface p-3"
      style={{ marginLeft: Math.min(t.depth, 3) * 12 }}
    >
      <header className="flex items-center gap-2">
        <Link to={teamPaths(ws, t.key).all as "/"} className="text-sm font-medium hover:underline">
          {t.name}
        </Link>
        <span className="text-xs text-muted-foreground">{t.key}</span>
        {t.cycleProgress !== null && (
          <span className="ml-auto text-xs text-muted-foreground">
            Cycle {Math.round(t.cycleProgress * 100)}%
          </span>
        )}
      </header>
      <Counts c={t.counts} />
      <Health h={t.projects} />
    </article>
  );
}

/** Firmen-Dashboard (docs/14 §2–3): Kachel je Team, Rollup-Schalter, Projekt-Health, Initiativen, gefährdete Issues. */
export function OverviewPage({ ws }: { ws: string }) {
  const [rollup, setRollup] = useState(true);
  const q = useOverview(rollup);
  useEffect(() => {
    document.title = "Overview · Lineal";
  }, []);
  const p = workspacePaths(ws);

  let body;
  if (q.isLoading) body = <ListSkeleton rows={5} />;
  else if (q.isError || !q.data)
    body = (
      <ErrorState
        error={q.error}
        title="Could not load overview"
        onRetry={() => void q.refetch()}
      />
    );
  else if (q.data.teams.length === 0) body = <EmptyState title="No teams yet" />;
  else {
    const d = q.data;
    body = (
      <div className="flex flex-col gap-6 p-4">
        <section aria-label="Totals" className="rounded-lg border border-border p-3">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">All teams</h2>
          <Counts c={d.totals} />
          <div className="mt-2">
            <Health h={d.projects} />
          </div>
        </section>
        <section aria-label="Teams" className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {d.teams.map((t) => (
            <TeamTile key={t.teamId} t={t} ws={ws} />
          ))}
        </section>
        <section aria-label="Initiatives">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">Initiatives</h2>
          {d.initiatives.length === 0 ? (
            <p className="text-sm text-muted-foreground">No initiatives.</p>
          ) : (
            <ul className="divide-y divide-border rounded-lg border border-border">
              {d.initiatives.map((i) => (
                <li key={i.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
                  <Link
                    to={`/${ws}/initiative/${i.identifier}/overview` as "/"}
                    className="hover:underline"
                  >
                    {i.name}
                  </Link>
                  <span className="text-xs text-muted-foreground">{i.status}</span>
                  <span className="ml-auto text-xs text-muted-foreground tabular-nums">
                    {i.completedProjectCount} / {i.projectCount} projects
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section aria-label="At risk">
          <h2 className="mb-2 text-xs font-medium text-muted-foreground">
            Overdue and SLA at risk
          </h2>
          {d.atRisk.length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing at risk.</p>
          ) : (
            <ul className="divide-y divide-border rounded-lg border border-border">
              {d.atRisk.map((i) => (
                <li key={i.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
                  <Link to={`/${ws}/issue/${i.identifier}` as "/"} className="hover:underline">
                    <span className="text-muted-foreground">{i.identifier}</span> {i.title}
                  </Link>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {i.reasons.join(", ")}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-3 border-b border-border px-3">
        <h1 className="text-sm font-medium">Overview</h1>
        <label className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={rollup}
            onChange={(e) => setRollup(e.target.checked)}
            aria-label="Include sub-teams"
          />
          Include sub-teams
        </label>
        <Link to={p.workload as "/"} className="text-xs text-muted-foreground hover:underline">
          Workload
        </Link>
        <Link to={p.cycles as "/"} className="text-xs text-muted-foreground hover:underline">
          Cycles
        </Link>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
