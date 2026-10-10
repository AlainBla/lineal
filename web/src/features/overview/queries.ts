/**
 * Daten der firmenweiten Sicht (docs/14): Overview, Workload, aktive Cycles.
 * Das Rollup über Sub-Teams rechnet der Server (`includeSubTeams`); der Client reicht nur den Schalter durch.
 */
import { useQuery } from "@tanstack/react-query";

import { fetchIssues } from "@/features/issues/queries";
import type { ActiveCycle, WorkloadRowGql, WorkspaceOverviewGql } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const overviewKeys = {
  overview: (includeSubTeams: boolean, period: string) =>
    ["overview", "dashboard", includeSubTeams, period] as const,
  workload: (cycle: string) => ["overview", "workload", cycle] as const,
  activeCycles: ["overview", "active-cycles"] as const,
};

const COUNTS = `openIssues inProgress triage triageOldestAt completed completedPoints overdue slaAtRisk slaBreached`;
const HEALTH = `onTrack atRisk offTrack noUpdate total`;

export async function fetchOverview(
  includeSubTeams: boolean,
  period: string,
  signal?: AbortSignal,
): Promise<WorkspaceOverviewGql> {
  const res = await gqlRequest<
    { workspaceOverview: WorkspaceOverviewGql },
    { includeSubTeams: boolean; period: string }
  >(
    `query WorkspaceOverview($includeSubTeams: Boolean, $period: String) {
      workspaceOverview(includeSubTeams: $includeSubTeams, period: $period) {
        period includeSubTeams
        teams { teamId key name parentId depth cycleProgress counts { ${COUNTS} } projects { ${HEALTH} } }
        totals { ${COUNTS} }
        projects { ${HEALTH} }
        initiatives { id identifier name status health projectCount completedProjectCount }
        atRisk { id identifier title teamKey dueDate slaBreachesAt reasons }
      }
    }`,
    { includeSubTeams, period },
    { signal },
  );
  return res.workspaceOverview;
}

export function useOverview(includeSubTeams: boolean, period = "7d") {
  return useQuery({
    queryKey: overviewKeys.overview(includeSubTeams, period),
    queryFn: ({ signal }) => fetchOverview(includeSubTeams, period, signal),
  });
}

export function useWorkload(cycle: "current" | "next") {
  return useQuery({
    queryKey: overviewKeys.workload(cycle),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workload: WorkloadRowGql[] }, { cycle: string }>(
          `query WorkspaceWorkload($cycle: String) {
            workload(cycle: $cycle) {
              user { id name displayName } totalOpenIssues totalPoints
              perTeam { teamId teamKey teamName openIssues points }
            }
          }`,
          { cycle },
          { signal },
        )
      ).workload,
  });
}

export function useActiveCycles() {
  return useQuery({
    queryKey: overviewKeys.activeCycles,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ activeCycles: ActiveCycle[] }>(
          `query WorkspaceActiveCycles {
            activeCycles {
              teamId teamKey teamName cycleId number name startsAt endsAt remainingDays
              progress scope completedScope inProgressScope scopeChange
            }
          }`,
          undefined,
          { signal },
        )
      ).activeCycles,
  });
}

/** Alle sichtbaren Issues (Workspace-Issues); Key unter `["issues"]`, damit optimistische Patches greifen. */
export function useWorkspaceIssues() {
  return useQuery({
    queryKey: ["issues", "workspace"] as const,
    queryFn: ({ signal }) => fetchIssues({}, signal),
  });
}
