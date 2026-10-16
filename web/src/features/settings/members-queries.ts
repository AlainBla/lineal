/** Daten der Mitglieder-Seite: Mitgliederliste und Einladungsverwaltung (GraphQL `invitations`, `inviteUsers`, …). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { Invitation } from "@/gql/graphql";
import { mergePlaceholder } from "@/features/import/api";
import { gqlRequest } from "@/lib/graphql";

import type { Member } from "./queries";

const INVITATION_FIELDS = "id email role teamIds inviterId status expiresAt lastSentAt createdAt";

export interface AdminMember {
  id: string;
  name: string;
  displayName: string;
  email: string;
  role: string;
  status: string;
  lastSeenAt: string | null;
  createdAt: string;
  teams: Array<{ key: string }>;
}

export const memberKeys = {
  members: ["settings", "workspace-member-list"] as const,
  adminMembers: ["settings", "workspace-admin-members"] as const,
  invitations: ["settings", "invitations"] as const,
};

export function useMemberList(enabled = true) {
  return useQuery({
    queryKey: memberKeys.members,
    enabled,
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

/** Admin-Mitgliederliste mit E-Mail, Rolle und Status (nur Owner/Admin; sonst `forbidden`). */
export function useAdminMembers(enabled: boolean) {
  return useQuery({
    queryKey: memberKeys.adminMembers,
    enabled,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workspaceMembers: AdminMember[] }>(
          `query SettingsAdminMembers {
            workspaceMembers(includeSuspended: true) {
              id name displayName email role status lastSeenAt createdAt teams { key }
            }
          }`,
          undefined,
          { signal },
        )
      ).workspaceMembers,
  });
}

/** Import-Platzhalter einem bestehenden aktiven User zuordnen (nur Owner/Admin; atomar im Server). */
export function usePlaceholderMerge() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (v: { placeholderId: string; targetUserId: string }) =>
      mergePlaceholder(v.placeholderId, v.targetUserId),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: memberKeys.adminMembers });
      void qc.invalidateQueries({ queryKey: memberKeys.members });
    },
  });
}

export function useMemberAdminMutations() {
  const qc = useQueryClient();
  const done = () => {
    void qc.invalidateQueries({ queryKey: memberKeys.adminMembers });
    void qc.invalidateQueries({ queryKey: memberKeys.members });
  };
  return {
    setRole: useMutation({
      mutationFn: async (v: { userId: string; role: string }) =>
        gqlRequest(
          `mutation SettingsUserRoleUpdate($userId: String!, $role: String!) {
            userRoleUpdate(userId: $userId, role: $role) { success member { id role } }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    suspend: useMutation({
      mutationFn: async (userId: string) =>
        gqlRequest(
          `mutation SettingsUserSuspend($userId: String!) {
            userSuspend(userId: $userId) { success member { id status } }
          }`,
          { userId },
        ),
      onSuccess: done,
    }),
    unsuspend: useMutation({
      mutationFn: async (userId: string) =>
        gqlRequest(
          `mutation SettingsUserUnsuspend($userId: String!) {
            userUnsuspend(userId: $userId) { success member { id status } }
          }`,
          { userId },
        ),
      onSuccess: done,
    }),
  };
}
