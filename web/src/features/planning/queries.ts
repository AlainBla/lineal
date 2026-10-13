/**
 * Daten der Planung (Projekte, Initiativen, Cycles): GraphQL-Operationen, Query-Keys, Hooks.
 * Alle Zugriffe der Komponenten laufen über diese Datei (WP15 ersetzt die Datenquellen dahinter).
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ISSUE_FIELDS, type IssueList } from "@/features/issues/queries";
import type {
  Cycle,
  DocumentConnection,
  Initiative,
  InitiativeUpdate,
  IssueConnection,
  Project,
  ProjectMilestone,
  ProjectUpdate,
} from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const planningKeys = {
  projects: (scope: string) => ["planning", "projects", scope] as const,
  project: (ref: string) => ["planning", "project", ref] as const,
  projectUpdates: (id: string) => ["planning", "project-updates", id] as const,
  initiatives: ["planning", "initiatives"] as const,
  initiative: (ref: string) => ["planning", "initiative", ref] as const,
  teamCycles: (team: string) => ["planning", "cycles", team] as const,
  cycle: (id: string) => ["planning", "cycle", id] as const,
};

const REF = `id name key`;
const PROJECT_FIELDS = `id identifier slugId name summary description icon color priority
  status { id name type color } health progress scope completedScope issueCount completedIssueCount
  startDate startDateResolution targetDate targetDateResolution startedAt completedAt archivedAt
  leadTeam { ${REF} } teams { ${REF} } lead { ${REF} } members { ${REF} } labels { ${REF} } initiatives { ${REF} }
  milestones { id name description targetDate sortOrder status progress issueCount completedIssueCount }
  openIssueCount createdAt updatedAt`;

const INITIATIVE_FIELDS = `id identifier slugId name summary description color icon status statusType priority health
  targetDate targetDateResolution startedAt completedAt owner { ${REF} } leadTeam { ${REF} } visibility
  parentInitiatives { ${REF} } subInitiatives { ${REF} } labels { ${REF} }
  projects { id identifier name status statusType health }
  healthRollup { color onTrack atRisk offTrack none } archivedAt createdAt updatedAt`;

const CYCLE_FIELDS = `id teamId number name description startsAt endsAt completedAt progress scope completedScope
  issueCount completedIssueCount scopeHistory completedScopeHistory inProgressScopeHistory
  isActive isPast isFuture isNext isPrevious capacity`;

// ---------- Projekte ----------

export interface ProjectsFilter {
  team?: string;
  includeSubTeams?: boolean;
  query?: string;
}

/** Seitengröße beim Nachladen aller Projekte (Cursor-Paginierung, B5.11). */
const PROJECT_PAGE_SIZE = 100;

export function useProjects(f: ProjectsFilter = {}) {
  return useQuery({
    queryKey: planningKeys.projects(JSON.stringify(f)),
    queryFn: async ({ signal }) => {
      const all: Project[] = [];
      let after: string | null = null;
      // Alle Seiten laden; die Obergrenze verhindert Endlosschleifen bei fehlerhaften Cursorn.
      for (let page = 0; page < 200; page++) {
        const res: { projectsPage: ProjectsPage } = await gqlRequest<
          { projectsPage: ProjectsPage },
          Record<string, unknown>
        >(
          `query PlanningProjects($team: String, $includeSubTeams: Boolean, $query: String, $first: Int, $after: String) {
            projectsPage(team: $team, includeSubTeams: $includeSubTeams, query: $query, first: $first, after: $after) {
              nodes { ${PROJECT_FIELDS} }
              pageInfo { hasNextPage endCursor }
            }
          }`,
          {
            team: f.team,
            includeSubTeams: f.includeSubTeams,
            query: f.query,
            first: PROJECT_PAGE_SIZE,
            after,
          },
          { signal },
        );
        all.push(...res.projectsPage.nodes);
        if (!res.projectsPage.pageInfo.hasNextPage || !res.projectsPage.pageInfo.endCursor) break;
        after = res.projectsPage.pageInfo.endCursor;
      }
      return all;
    },
  });
}

interface ProjectsPage {
  nodes: Project[];
  pageInfo: { hasNextPage: boolean; endCursor: string | null };
}

/** Erster Monat des Geschäftsjahres (0 = Januar) für Quartals-/Halbjahres-Auflösung. */
export function useFiscalMonth(): number {
  const q = useQuery({
    queryKey: ["planning", "fiscal-month"],
    staleTime: 5 * 60_000,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workspace: { fiscalYearStartMonth: number } }>(
          `query PlanningFiscalMonth { workspace { fiscalYearStartMonth } }`,
          undefined,
          { signal },
        )
      ).workspace.fiscalYearStartMonth,
  });
  return q.data ?? 0;
}

export function useProject(ref: string) {
  return useQuery({
    queryKey: planningKeys.project(ref),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ project: Project }, { id: string }>(
          `query PlanningProject($id: String!) { project(id: $id) { ${PROJECT_FIELDS} } }`,
          { id: ref },
          { signal },
        )
      ).project,
  });
}

export function useProjectUpdates(projectId: string | undefined) {
  return useQuery({
    queryKey: planningKeys.projectUpdates(projectId ?? ""),
    enabled: !!projectId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ projectUpdates: ProjectUpdate[] }, { projectId: string }>(
          `query PlanningProjectUpdates($projectId: String) {
            projectUpdates(projectId: $projectId, first: 50) {
              id body health isStale editedAt createdAt user { ${REF} }
            }
          }`,
          { projectId: projectId! },
          { signal },
        )
      ).projectUpdates,
  });
}

export type Health = "onTrack" | "atRisk" | "offTrack";
export const HEALTH_LABEL: Record<Health, string> = {
  onTrack: "On track",
  atRisk: "At risk",
  offTrack: "Off track",
};

export function usePostProjectUpdate(projectId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { body: string; health: Health }) =>
      gqlRequest<{ projectUpdateCreate: { success: boolean } }, Record<string, unknown>>(
        `mutation PlanningProjectUpdateCreate($projectId: String!, $body: String!, $health: String) {
          projectUpdateCreate(projectId: $projectId, body: $body, health: $health) { success }
        }`,
        { projectId, ...v },
      ),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: planningKeys.projectUpdates(projectId) });
      void qc.invalidateQueries({ queryKey: ["planning", "project"] });
      void qc.invalidateQueries({ queryKey: ["planning", "projects"] });
    },
  });
}

export function useCreateMilestone(projectId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { name: string; targetDate?: string }) =>
      gqlRequest<
        { projectMilestoneCreate: { projectMilestone: ProjectMilestone | null } },
        Record<string, unknown>
      >(
        `mutation PlanningMilestoneCreate($projectId: String!, $input: MilestoneInput!) {
          projectMilestoneCreate(projectId: $projectId, input: $input) { success }
        }`,
        { projectId, input },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["planning", "project"] }),
  });
}

export function useDeleteMilestone(projectId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(
        `mutation PlanningMilestoneDelete($projectId: String!, $id: String!) {
          projectMilestoneDelete(projectId: $projectId, id: $id) { success }
        }`,
        { projectId, id },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["planning", "project"] }),
  });
}

/** Ändert Felder eines Projekts (`state` = Statusname/-ID); optimistisch im Detail-Cache. */
export function useUpdateProject() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
      gqlRequest(
        `mutation PlanningProjectUpdate($id: String!, $input: ProjectInput!) {
          projectUpdate(id: $id, input: $input) { success }
        }`,
        v,
      ),
    onSettled: () => void qc.invalidateQueries({ queryKey: ["planning"] }),
  });
}

// ---------- Initiativen ----------

export function useInitiatives() {
  return useQuery({
    queryKey: planningKeys.initiatives,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ initiatives: Initiative[] }>(
          `query PlanningInitiatives { initiatives(first: 200) { ${INITIATIVE_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).initiatives,
  });
}

export function useInitiative(ref: string, enabled = true) {
  return useQuery({
    queryKey: planningKeys.initiative(ref),
    enabled,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ initiative: Initiative }, { id: string }>(
          `query PlanningInitiative($id: String!) { initiative(id: $id) { ${INITIATIVE_FIELDS} } }`,
          { id: ref },
          { signal },
        )
      ).initiative,
  });
}

// ---------- Cycles ----------

export function useTeamCycles(team: string) {
  return useQuery({
    queryKey: planningKeys.teamCycles(team),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ cycles: Cycle[] }, { team: string }>(
          `query PlanningCycles($team: String!) { cycles(teamId: $team) { ${CYCLE_FIELDS} } }`,
          { team },
          { signal },
        )
      ).cycles,
  });
}

/** Ein Cycle per UUID, oder `active`/`upcoming` (Auflösung über die Cycles des Teams). */
export function useCycle(team: string, ref: string) {
  return useQuery({
    queryKey: planningKeys.cycle(`${team}/${ref}`),
    queryFn: async ({ signal }) => {
      if (ref === "active" || ref === "upcoming") {
        const type = ref === "active" ? "current" : "next";
        const res = await gqlRequest<{ cycles: Cycle[] }, { team: string; type: string }>(
          `query PlanningCycleByType($team: String!, $type: String) { cycles(teamId: $team, type: $type) { ${CYCLE_FIELDS} } }`,
          { team, type },
          { signal },
        );
        return res.cycles[0] ?? null;
      }
      const res = await gqlRequest<{ cycle: Cycle }, { id: string }>(
        `query PlanningCycle($id: String!) { cycle(id: $id) { ${CYCLE_FIELDS} } }`,
        { id: ref },
        { signal },
      );
      return res.cycle;
    },
  });
}

// ---------- Issues eines Projekts/Cycles ----------

/** Issues nach Projekt oder Cycle; Key unter `["issues"]`, damit optimistische Patches (WP13) greifen. */
export function useScopedIssues(scope: { projectId?: string; cycleId?: string } | null) {
  return useQuery({
    queryKey: ["issues", "scoped", scope?.projectId ?? "", scope?.cycleId ?? ""] as const,
    enabled: !!scope,
    queryFn: async ({ signal }): Promise<IssueList> => {
      const res = await gqlRequest<{ issues: IssueConnection }, Record<string, unknown>>(
        `query PlanningScopedIssues($projectId: String, $cycleId: String) {
          issues(projectId: $projectId, cycleId: $cycleId, first: 250) {
            nodes { ${ISSUE_FIELDS} } hasNextPage endCursor
          }
        }`,
        { projectId: scope?.projectId, cycleId: scope?.cycleId },
        { signal },
      );
      return {
        issues: res.issues.nodes.filter((i) => !i.trashed && i.stateType !== "triage"),
        truncated: res.issues.hasNextPage,
      };
    },
  });
}

/** Ändert Felder einer Initiative (z. B. `targetDate`); Rechte prüft der Server. */
export function useUpdateInitiative() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
      gqlRequest(
        `mutation PlanningInitiativeUpdate($id: String!, $input: InitiativeInput!) {
          initiativeUpdate(id: $id, input: $input) { success }
        }`,
        v,
      ),
    onSettled: () => void qc.invalidateQueries({ queryKey: ["planning"] }),
  });
}

export function useInitiativeUpdates(initiativeId: string | undefined) {
  return useQuery({
    queryKey: ["planning", "initiative-updates", initiativeId ?? ""] as const,
    enabled: !!initiativeId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ initiativeUpdates: InitiativeUpdate[] }, { initiativeId: string }>(
          `query PlanningInitiativeUpdates($initiativeId: String) {
            initiativeUpdates(initiativeId: $initiativeId, first: 50) {
              id body health isStale editedAt createdAt user { ${REF} }
            }
          }`,
          { initiativeId: initiativeId! },
          { signal },
        )
      ).initiativeUpdates,
  });
}

export function usePostInitiativeUpdate(initiativeId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { body: string; health: Health }) =>
      gqlRequest(
        `mutation PlanningInitiativeUpdateCreate($initiativeId: String!, $body: String!, $health: String) {
          initiativeUpdateCreate(initiativeId: $initiativeId, body: $body, health: $health) { success }
        }`,
        { initiativeId, ...v },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["planning"] }),
  });
}

export function useInitiativeDocuments(initiativeId: string | undefined) {
  return useQuery({
    queryKey: ["planning", "initiative-documents", initiativeId ?? ""] as const,
    enabled: !!initiativeId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ documents: DocumentConnection }, { initiativeId: string }>(
          `query PlanningInitiativeDocuments($initiativeId: String) {
            documents(initiativeId: $initiativeId, first: 100) {
              nodes { id slugId title icon updatedAt }
            }
          }`,
          { initiativeId: initiativeId! },
          { signal },
        )
      ).documents.nodes,
  });
}
