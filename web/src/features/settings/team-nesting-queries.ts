/** Daten für „Recently deleted teams“ und den Nesting-Wizard (docs/02 §3.3). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { gqlRequest } from "@/lib/graphql";

import { refreshTeams, type TeamRow } from "./teams-queries";

export interface DeletedTeam {
  id: string;
  key: string;
  name: string;
  displayName: string;
  parentId: string | null;
  trashedAt: string;
  restorableUntil: string;
  subTeamCount: number;
}

export const deletedTeamsKey = ["settings", "teams", "deleted"] as const;

/** Nur für Admins sinnvoll; der Server liefert sonst eine leere Liste. */
export function useDeletedTeams(enabled: boolean) {
  return useQuery({
    queryKey: deletedTeamsKey,
    enabled,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ deletedTeams: DeletedTeam[] }>(
          `query SettingsDeletedTeams {
            deletedTeams { id key name displayName parentId trashedAt restorableUntil subTeamCount }
          }`,
          undefined,
          { signal },
        )
      ).deletedTeams,
  });
}

export function useRestoreTeam() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(
        `mutation SettingsTeamUnarchive($id: String!) { teamUnarchive(id: $id) { success } }`,
        { id },
      ),
    onSuccess: async () => {
      void qc.invalidateQueries({ queryKey: deletedTeamsKey });
      await refreshTeams(qc);
    },
  });
}

/** Ein Konflikt der Vorschau; `details` hängt von `kind` ab. */
export interface NestingConflict {
  kind: "membership" | "visibility" | "workflowState" | "label" | "cycle" | "estimate" | string;
  message: string;
  details: Record<string, unknown>;
}

export interface StateMappingEntry {
  id: string;
  name: string;
  type: string;
  targetId: string;
  targetName: string;
}

export function usePreviewNesting(teamId: string, parentId: string | null, enabled: boolean) {
  return useQuery({
    queryKey: ["settings", "teams", "nesting-preview", teamId, parentId],
    enabled,
    retry: false,
    gcTime: 0,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<
          { previewTeamNesting: { conflicts: NestingConflict[] } },
          { teamId: string; parentId: string | null }
        >(
          `query SettingsNestingPreview($teamId: String!, $parentId: String) {
            previewTeamNesting(teamId: $teamId, parentId: $parentId) { conflicts { kind message details } }
          }`,
          { teamId, parentId },
          { signal },
        )
      ).previewTeamNesting.conflicts,
  });
}

export interface NestOptionsInput {
  addMissingMembersToParent: boolean;
  downgradeVisibility: boolean;
  inheritWorkflowStatuses: boolean;
  inheritIssueEstimation: boolean;
  stateMapping: Array<{ fromStateId: string; toStateId: string }>;
}

export function useNestTeam() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: {
      teamId: string;
      parentId: string | null;
      options?: NestOptionsInput;
    }) =>
      gqlRequest(
        `mutation SettingsNestTeam($teamId: String!, $parentId: String, $options: NestTeamOptions) {
          nestTeam(teamId: $teamId, parentId: $parentId, options: $options) { success }
        }`,
        v,
      ),
    onSuccess: async () => {
      await refreshTeams(qc);
    },
  });
}

export interface ParentState {
  id: string;
  name: string;
  type: string;
}

/** Status des Parents für die Zuordnung (ohne Triage). */
export function useParentStates(parentId: string | null) {
  return useQuery({
    queryKey: ["settings", "teams", "parent-states", parentId],
    enabled: !!parentId,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workflowStates: ParentState[] }, { teamId: string }>(
          `query SettingsParentStates($teamId: String!) {
            workflowStates(teamId: $teamId) { id name type }
          }`,
          { teamId: parentId! },
          { signal },
        )
      ).workflowStates.filter((s) => s.type !== "triage"),
  });
}

/** Teams, unter die `team` gehängt werden darf: aktiv, nicht es selbst, kein Nachfahre. */
export function parentCandidates(team: TeamRow, all: TeamRow[]): TeamRow[] {
  const blocked = new Set([team.id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const t of all) {
      if (t.parentId && blocked.has(t.parentId) && !blocked.has(t.id)) {
        blocked.add(t.id);
        grew = true;
      }
    }
  }
  return all
    .filter((t) => !blocked.has(t.id) && !t.retiredAt)
    .sort((a, b) => a.name.localeCompare(b.name));
}
