/**
 * Daten der Settings (Team und Workspace). Mutationen invalidieren `["settings"]` sowie die Team-Metadaten
 * der Issue-Screens (`["team-meta"]`), damit Status, Labels und Cycles sofort überall aktuell sind.
 */
import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";

import type { IssueLabel, Organization, Team, WorkflowState } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

import { refreshTeams } from "./teams-queries";

export const settingsKeys = {
  team: (key: string) => ["settings", "team", key] as const,
  members: (key: string) => ["settings", "members", key] as const,
  states: (teamId: string) => ["settings", "states", teamId] as const,
  labels: (teamId: string) => ["settings", "labels", teamId] as const,
  workspace: ["settings", "workspace"] as const,
};

const TEAM_FIELDS = `id key name displayName parentId description icon color visibility allMembersCanJoin timezone triageEnabled
  cyclesEnabled issueEstimationType inheritWorkflowStatuses retiredAt`;

function refresh(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: ["settings"] });
  void qc.invalidateQueries({ queryKey: ["team-meta"] });
  void qc.invalidateQueries({ queryKey: ["planning"] });
}

export function useSettingsTeam(key: string) {
  return useQuery({
    queryKey: settingsKeys.team(key),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ team: Team }, { id: string }>(
          `query SettingsTeam($id: String!) { team(id: $id) { ${TEAM_FIELDS} } }`,
          { id: key },
          { signal },
        )
      ).team,
  });
}

export function useUpdateTeam(teamId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: Record<string, unknown>) =>
      gqlRequest(
        `mutation SettingsTeamUpdate($id: String!, $input: TeamUpdateInput!) {
          teamUpdate(id: $id, input: $input) { success }
        }`,
        { id: teamId, input },
      ),
    onSuccess: async () => {
      refresh(qc);
      await refreshTeams(qc);
    },
  });
}

export interface CycleSettings {
  cyclesEnabled?: boolean;
  cycleDuration?: number;
  cycleCooldownTime?: number;
  cycleStartDay?: number;
  upcomingCycleCount?: number;
}

export function useUpdateCycleSettings(teamId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: CycleSettings) =>
      gqlRequest(
        `mutation SettingsCyclesUpdate($teamId: String!, $input: TeamCyclesInput!) {
          teamCyclesUpdate(teamId: $teamId, input: $input) { success }
        }`,
        { teamId, input },
      ),
    onSuccess: () => refresh(qc),
  });
}

// ---------- Mitglieder ----------

export interface Member {
  id: string;
  name: string;
  displayName: string;
}

export function useTeamMembers(teamKey: string) {
  return useQuery({
    queryKey: settingsKeys.members(teamKey),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ users: Member[] }, { team: string }>(
          `query SettingsTeamMembers($team: String) { users(team: $team, first: 200) { id name displayName } }`,
          { team: teamKey },
          { signal },
        )
      ).users,
  });
}

export function useWorkspaceMembers(enabled: boolean) {
  return useQuery({
    queryKey: ["settings", "workspace-members"] as const,
    enabled,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ users: Member[] }>(
          `query SettingsWorkspaceMembers { users(first: 200) { id name displayName } }`,
          undefined,
          { signal },
        )
      ).users,
  });
}

export function useMembership(teamId: string) {
  const qc = useQueryClient();
  return {
    add: useMutation({
      mutationFn: async (userId: string) =>
        gqlRequest(
          `mutation SettingsMemberAdd($teamId: String!, $userId: String!) {
            teamMembershipCreate(teamId: $teamId, userId: $userId) { success }
          }`,
          { teamId, userId },
        ),
      onSuccess: () => refresh(qc),
    }),
    remove: useMutation({
      mutationFn: async (userId: string) =>
        gqlRequest(
          `mutation SettingsMemberRemove($teamId: String!, $userId: String!) {
            teamMembershipDelete(teamId: $teamId, userId: $userId) { success }
          }`,
          { teamId, userId },
        ),
      onSuccess: () => refresh(qc),
    }),
  };
}

// ---------- Workflow ----------

export function useWorkflowStates(teamId: string | undefined) {
  return useQuery({
    queryKey: settingsKeys.states(teamId ?? ""),
    enabled: !!teamId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workflowStates: WorkflowState[] }, { teamId: string }>(
          `query SettingsStates($teamId: String!) {
            workflowStates(teamId: $teamId) { id teamId name type color description position inheritedFromId isDefault }
          }`,
          { teamId: teamId! },
          { signal },
        )
      ).workflowStates,
  });
}

export function useWorkflowMutations() {
  const qc = useQueryClient();
  const done = () => refresh(qc);
  return {
    create: useMutation({
      mutationFn: async (input: {
        teamId: string;
        name: string;
        type: string;
        color: string;
        description?: string;
      }) =>
        gqlRequest(
          `mutation SettingsStateCreate($input: WorkflowStateCreateInput!) {
            workflowStateCreate(input: $input) { success }
          }`,
          { input },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
        gqlRequest(
          `mutation SettingsStateUpdate($id: String!, $input: WorkflowStateUpdateInput!) {
            workflowStateUpdate(id: $id, input: $input) { success }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    archive: useMutation({
      mutationFn: async (v: { id: string; targetStateId?: string }) =>
        gqlRequest(
          `mutation SettingsStateArchive($id: String!, $targetStateId: String) {
            workflowStateArchive(id: $id, targetStateId: $targetStateId) { success }
          }`,
          v,
        ),
      onSuccess: done,
    }),
  };
}

// ---------- Labels ----------

export function useTeamLabels(teamId: string | undefined) {
  return useQuery({
    queryKey: settingsKeys.labels(teamId ?? ""),
    enabled: !!teamId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ issueLabels: IssueLabel[] }, { teamId: string }>(
          `query SettingsLabels($teamId: String) {
            issueLabels(teamId: $teamId, includeGroups: true) {
              id name color description teamId parentId isGroup groupType retiredAt
            }
          }`,
          { teamId: teamId! },
          { signal },
        )
      ).issueLabels,
  });
}

export function useLabelMutations() {
  const qc = useQueryClient();
  const done = () => refresh(qc);
  return {
    create: useMutation({
      mutationFn: async (input: {
        name: string;
        color?: string;
        teamId?: string;
        isGroup?: boolean;
        parentId?: string;
      }) =>
        gqlRequest(
          `mutation SettingsLabelCreate($input: IssueLabelCreateInput!) {
            issueLabelCreate(input: $input) { success }
          }`,
          { input },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
        gqlRequest(
          `mutation SettingsLabelUpdate($id: String!, $input: IssueLabelUpdateInput!) {
            issueLabelUpdate(id: $id, input: $input) { success }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    retire: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsLabelRetire($id: String!) { issueLabelRetire(id: $id) { success } }`,
          { id },
        ),
      onSuccess: done,
    }),
  };
}

// ---------- Workspace ----------

export function useWorkspaceSettings() {
  return useQuery({
    queryKey: settingsKeys.workspace,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workspace: Organization }>(
          `query SettingsWorkspace { workspace { id name urlKey fiscalYearStartMonth initiativesEnabled logoUrl } }`,
          undefined,
          { signal },
        )
      ).workspace,
  });
}

export function useUpdateWorkspace() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: Record<string, unknown>) =>
      gqlRequest(
        `mutation SettingsWorkspaceUpdate($input: WorkspaceUpdateInput!) {
          workspaceUpdate(input: $input) { success }
        }`,
        { input },
      ),
    onSuccess: () => {
      refresh(qc);
      void qc.invalidateQueries({ queryKey: ["session"] });
    },
  });
}
