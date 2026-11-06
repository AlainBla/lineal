/** Alle Meilensteine über die Projekte hinweg (docs/08), nach Datum und ohne Abgeschlossene. */
import { Link } from "@tanstack/react-router";
import { Diamond } from "lucide-react";
import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Project, ProjectMilestone } from "@/gql/graphql";
import { cn } from "@/lib/cn";

import { CalendarFeedButton } from "./calendar-feed";
import { useProjects } from "./queries";
import {
  MILESTONE_COLOR,
  MilestoneBar,
  milestonePath,
  PageHeader,
  shiftLabel,
  shortDate,
  targetShift,
} from "./ui";

const SOON_DAYS = 30;

interface Row {
  project: Project;
  milestone: ProjectMilestone;
}

export interface MilestoneGroup {
  key: string;
  label: string;
  rows: Row[];
}

/** Gruppen in fester Reihenfolge; leere fallen weg. */
export function groupMilestones(
  projects: Project[],
  showCompleted: boolean,
  now = Date.now(),
): MilestoneGroup[] {
  const groups: MilestoneGroup[] = [
    { key: "overdue", label: "Overdue", rows: [] },
    { key: "soon", label: `Next ${SOON_DAYS} days`, rows: [] },
    { key: "later", label: "Later", rows: [] },
    { key: "none", label: "No target date", rows: [] },
    { key: "completed", label: "Completed", rows: [] },
  ];
  const soon = now + SOON_DAYS * 86_400_000;
  const byKey = new Map(groups.map((g) => [g.key, g.rows]));
  for (const project of projects) {
    for (const milestone of project.milestones) {
      const row = { project, milestone };
      if (milestone.status === "done") {
        if (showCompleted) byKey.get("completed")!.push(row);
      } else if (milestone.status === "overdue") byKey.get("overdue")!.push(row);
      else if (!milestone.targetDate) byKey.get("none")!.push(row);
      else if (Date.parse(milestone.targetDate) <= soon) byKey.get("soon")!.push(row);
      else byKey.get("later")!.push(row);
    }
  }
  const date = (r: Row) => r.milestone.targetDate ?? "9999";
  for (const g of groups) g.rows.sort((a, b) => date(a).localeCompare(date(b)));
  return groups.filter((g) => g.rows.length > 0);
}

export function MilestonesPage({ ws }: { ws: string }) {
  const q = useProjects({});
  const [showCompleted, setShowCompleted] = useState(false);
  const groups = useMemo(
    () => groupMilestones(q.data ?? [], showCompleted),
    [q.data, showCompleted],
  );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Milestones"
        docTitle="Milestones"
        actions={
          <>
            <CalendarFeedButton />
            <Button
              variant="outline"
              size="sm"
              aria-pressed={showCompleted}
              onClick={() => setShowCompleted((s) => !s)}
            >
              {showCompleted ? "Hide completed" : "Show completed"}
            </Button>
          </>
        }
      />
      <div className="min-h-0 flex-1 overflow-auto p-4">
        {q.isLoading && <ListSkeleton rows={6} />}
        {q.isError && (
          <ErrorState
            title="Could not load milestones"
            error={q.error}
            onRetry={() => void q.refetch()}
          />
        )}
        {q.isSuccess && groups.length === 0 && (
          <p className="text-sm text-muted-foreground">No open milestones.</p>
        )}
        {groups.length > 0 && (
          <div className="overflow-x-auto rounded-lg border border-border">
            <table className="w-full min-w-[720px] text-sm">
              <thead className="text-left text-xs text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-3 py-2 font-medium">Milestone</th>
                  <th className="px-3 py-2 font-medium">Project</th>
                  <th className="px-3 py-2 font-medium">Target date</th>
                  <th className="px-3 py-2 font-medium">Issues</th>
                  <th className="w-1/5 px-3 py-2 font-medium">
                    <span className="sr-only">Progress</span>
                  </th>
                </tr>
              </thead>
              {groups.map((g) => (
                <tbody key={g.key} aria-label={g.label} className="divide-y divide-border">
                  <tr className="bg-surface">
                    <th colSpan={5} className="px-3 py-1.5 text-left font-medium">
                      {g.label}{" "}
                      <span className="font-normal text-muted-foreground">{g.rows.length}</span>
                    </th>
                  </tr>
                  {g.rows.map(({ project, milestone: m }) => {
                    const shift = targetShift(m);
                    return (
                      <tr key={m.id} className="hover:bg-surface-hover">
                        <td className="px-3 py-2">
                          <span className="flex items-center gap-2">
                            <Diamond
                              className={cn(
                                "size-3.5 shrink-0 fill-current",
                                MILESTONE_COLOR[m.status],
                              )}
                            />
                            <Link
                              to={milestonePath(ws, project.identifier, m.id) as "/"}
                              className="truncate font-medium hover:underline"
                            >
                              {m.name}
                            </Link>
                          </span>
                        </td>
                        <td className="px-3 py-2 text-muted-foreground">
                          <Link
                            to={`/${ws}/project/${project.identifier}/overview` as "/"}
                            className="hover:underline"
                          >
                            {project.name}
                          </Link>
                        </td>
                        <td className="px-3 py-2 whitespace-nowrap text-muted-foreground">
                          {shortDate(m.targetDate)}
                          {shift !== null && (
                            <span className="ml-1 text-amber-500">{shiftLabel(shift)}</span>
                          )}
                        </td>
                        <td className="px-3 py-2 whitespace-nowrap tabular-nums">
                          {m.completedIssueCount} of {m.issueCount}
                        </td>
                        <td className="px-3 py-2">
                          <MilestoneBar
                            done={m.completedIssueCount}
                            started={m.startedIssueCount}
                            total={m.issueCount}
                          />
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              ))}
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
