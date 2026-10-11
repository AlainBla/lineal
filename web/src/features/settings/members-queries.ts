/** Daten der Mitglieder-Seite: Mitgliederliste und Einladungsverwaltung (GraphQL `invitations`, `inviteUsers`, …). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { Invitation } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

import type { Member } from "./queries";

const INVITATION_FIELDS = "id email role teamIds inviterId status expiresAt lastSentAt createdAt";

export const memberKeys = {
  members: ["settings", "workspace-member-list"] as const,
  invitations: ["settings", "invitations"] as const,
};

export function useMemberList() {
  return useQuery({
    queryKey: memberKeys.members,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ users: Member[] }>(
          `query SettingsMemberList { users(first: 200) { id name displayName } }`,
          undefined,
          { signal },
        )
      ).users,
  });
}

/** Offene Einladungen; schlägt für Rollen ohne Recht `invitationsRole` mit `forbidden` fehl. */
export function useInvitations(enabled: boolean) {
  return useQuery({
    queryKey: memberKeys.invitations,
    enabled,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ invitations: Invitation[] }>(
          `query SettingsInvitations { invitations(pendingOnly: true) { ${INVITATION_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).invitations,
  });
}

export interface InviteInput {
  emails: string[];
  role: string;
  teamIds: string[];
}

export function useInvitationMutations() {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: memberKeys.invitations });
  return {
    invite: useMutation({
      mutationFn: async (input: InviteInput) =>
        gqlRequest(
          `mutation SettingsInviteUsers($input: InviteUsersGqlInput!) {
            inviteUsers(input: $input) { success invitations { id } }
          }`,
          { input },
        ),
      onSuccess: done,
    }),
    resend: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsInvitationResend($id: String!) { invitationResend(id: $id) { success } }`,
          { id },
        ),
      onSuccess: done,
    }),
    revoke: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsInvitationRevoke($id: String!) { invitationRevoke(id: $id) { success } }`,
          { id },
        ),
      onSuccess: done,
    }),
  };
}
