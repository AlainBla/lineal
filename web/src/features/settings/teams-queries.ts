/** Daten der Teams-Übersicht und der Team-Verwaltung (Anlegen, stilllegen, löschen). */
import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";

import { SESSION_QUERY_KEY } from "@/app/auth";
import type { Team } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export interface TeamRow extends Pick<
  Team,
  | "id"
  | "key"
  | "name"
  | "parentId"
  | "icon"
  | "color"
  | "visibility"
  | "allMembersCanJoin"
  | "retiredAt"
  | "createdAt"
> {
  /** `null`, wenn der Server die Mitglieder nicht preisgibt. */
  memberCount?: number | null;
  /** Beziehung des Aufrufers zum Team (Anzeige von „Join“; der Server prüft erneut). */
  viewer?: { isMember: boolean; canJoin: boolean } | null;
}

export const teamsKeys = { overview: ["settings", "teams"] as const };

/** Session (Sidebar, Team-Navigation) neu laden – erst die Session, dann die davon abhängigen Daten. */
export async function refreshTeams(qc: QueryClient) {
  await qc.invalidateQueries({ queryKey: SESSION_QUERY_KEY });
  void qc.invalidateQueries({ queryKey: ["sidebar-data"] });
  void qc.invalidateQueries({ queryKey: ["settings"] });
  void qc.invalidateQueries({ queryKey: ["team-meta"] });
}

/** Alle sichtbaren Teams inkl. stillgelegter. */
export function useTeamsOverview(enabled = true) {
  return useQuery({
    queryKey: teamsKeys.overview,
    enabled,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ teams: TeamRow[] }>(
          `query SettingsTeams {
            teams(includeArchived: true, first: 250) {
              id key name parentId icon color visibility allMembersCanJoin retiredAt createdAt memberCount
              viewer { isMember canJoin }
            }
          }`,
          undefined,
          { signal },
        )
      ).teams,
  });
}

/** Selbstbeitritt aus der Teams-Liste (`canJoin`); der Server prüft die Policy erneut. */
export function useJoinTeam() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { teamId: string; userId: string }) =>
      gqlRequest(
        `mutation SettingsTeamsListJoin($teamId: String!, $userId: String!) {
          teamMembershipCreate(teamId: $teamId, userId: $userId) { success }
        }`,
        v,
      ),
    onSuccess: async () => {
      await refreshTeams(qc);
      void qc.invalidateQueries({ queryKey: teamsKeys.overview });
    },
  });
}

export interface CreateTeamVars {
  name: string;
  key: string;
  icon?: string;
  color?: string;
  parentId?: string;
  visibility?: string;
  copySettingsFromTeamId?: string;
}

export function useCreateTeam() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: CreateTeamVars) =>
      (
        await gqlRequest<
          { teamCreate: { team: { id: string; key: string } | null } },
          { input: CreateTeamVars }
        >(
          `mutation SettingsTeamCreate($input: TeamCreateInput!) {
            teamCreate(input: $input) { success team { id key } }
          }`,
          { input },
        )
      ).teamCreate.team,
    onSuccess: () => refreshTeams(qc),
  });
}

export function useTeamLifecycle(teamId: string) {
  const qc = useQueryClient();
  const done = () => refreshTeams(qc);
  return {
    retire: useMutation({
      mutationFn: async (handleSubTeamsOnRetirement?: "retire" | "unnest") =>
        gqlRequest(
          `mutation SettingsTeamRetire($id: String!, $handleSubTeamsOnRetirement: String) {
            teamRetire(id: $id, handleSubTeamsOnRetirement: $handleSubTeamsOnRetirement) { success }
          }`,
          { id: teamId, handleSubTeamsOnRetirement },
        ),
      onSuccess: done,
    }),
    unretire: useMutation({
      mutationFn: async () =>
        gqlRequest(
          `mutation SettingsTeamUnretire($id: String!) { teamUnretire(id: $id) { success } }`,
          { id: teamId },
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: async (handleSubTeams?: "delete" | "unnest") =>
        gqlRequest(
          `mutation SettingsTeamDelete($id: String!, $handleSubTeams: String) {
            teamDelete(id: $id, handleSubTeams: $handleSubTeams) { success }
          }`,
          { id: teamId, handleSubTeams },
        ),
      onSuccess: done,
    }),
  };
}
