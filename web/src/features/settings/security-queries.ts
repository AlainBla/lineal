/** Security-Settings des Workspace (GraphQL `workspaceSecuritySettings`, `workspaceSecuritySettingsUpdate`). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { gqlRequest } from "@/lib/graphql";

export const ROLE_FIELDS = [
  "teamCreationRole",
  "invitationsRole",
  "adminManagementRole",
  "labelManagementRole",
  "templateManagementRole",
  "personalApiKeysRole",
  "apiSettingsRole",
  "workspaceInitiativesRole",
  "pinnedViewManagementRole",
  "importRole",
] as const;

export type RoleField = (typeof ROLE_FIELDS)[number];

export type SecuritySettings = Record<RoleField, string> & { allowedInviteDomains: string[] };

/** Verständliche Frage je Recht (Reihenfolge wie in docs/02 §6.3). */
export const ROLE_QUESTIONS: Record<RoleField, string> = {
  teamCreationRole: "Who can create teams?",
  invitationsRole: "Who can invite members?",
  adminManagementRole: "Who can grant or remove the admin role?",
  labelManagementRole: "Who can manage workspace labels?",
  templateManagementRole: "Who can manage workspace templates?",
  personalApiKeysRole: "Who can create personal API keys?",
  apiSettingsRole: "Who can manage webhooks and OAuth apps?",
  workspaceInitiativesRole: "Who can create workspace initiatives?",
  pinnedViewManagementRole: "Who can pin views to workspace pages?",
  importRole: "Who can import data?",
};

export const ROLE_OPTIONS: Record<string, string> = {
  user: "All members",
  admin: "Admins and owners",
  owner: "Owners only",
};

/** Zerlegt Komma-/Zeilen-/Leerzeichen-getrennte Domains; entfernt Duplikate (ohne Groß-/Kleinschreibung). */
export function parseDomains(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/[\s,;]+/)) {
    const d = raw.trim().replace(/^@/, "");
    if (!d || seen.has(d.toLowerCase())) continue;
    seen.add(d.toLowerCase());
    out.push(d);
  }
  return out;
}

export const securityKey = ["settings", "security"] as const;

const FIELDS = `${ROLE_FIELDS.join(" ")} allowedInviteDomains`;

/** Liefert `null`, wenn der Server keine Settings liefert (kein Fehlerzustand für reine Hinweise). */
export function useSecuritySettings(enabled = true) {
  return useQuery({
    queryKey: securityKey,
    enabled,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ workspaceSecuritySettings?: SecuritySettings }>(
          `query SettingsSecurity { workspaceSecuritySettings { ${FIELDS} } }`,
          undefined,
          { signal },
        )
      ).workspaceSecuritySettings ?? null,
  });
}

export type SecurityUpdate = Partial<Record<RoleField, string>> & {
  allowedInviteDomains?: string[];
};

export function useUpdateSecuritySettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: SecurityUpdate) =>
      (
        await gqlRequest<
          { workspaceSecuritySettingsUpdate: { securitySettings: SecuritySettings } },
          { input: SecurityUpdate }
        >(
          `mutation SettingsSecurityUpdate($input: SecuritySettingsUpdateInput!) {
            workspaceSecuritySettingsUpdate(input: $input) { success securitySettings { ${FIELDS} } }
          }`,
          { input },
        )
      ).workspaceSecuritySettingsUpdate.securitySettings,
    onSuccess: (data) => qc.setQueryData(securityKey, data),
  });
}
