/**
 * Daten der Template-Verwaltung (docs/06 §2) und der Workspace-Labels. Mutationen invalidieren
 * `["settings"]` (Listen) und `["team-meta"]` (Labels in den Issue-Screens).
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { IssueLabel, Template } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export type TemplateKind = "issue" | "project" | "document";
export type DefaultSlot = "forMembers" | "forNonMembers" | "project";

export interface TemplateDefaults {
  forMembersId: string | null;
  forNonMembersId: string | null;
  projectId: string | null;
}

export const templateKeys = {
  list: (teamId: string | null) => ["settings", "templates", teamId ?? "workspace"] as const,
  defaults: (teamId: string) => ["settings", "template-defaults", teamId] as const,
  workspaceLabels: ["settings", "workspace-labels"] as const,
};

const TEMPLATE_FIELDS = `id name type description icon color teamId teamKey templateData sortOrder inheritedFromId isForm`;

/** Templates eines Teams (eigene, geerbte, Workspace) bzw. – ohne Team – nur die des Workspace. */
export function useTemplates(teamId: string | null) {
  return useQuery({
    queryKey: templateKeys.list(teamId),
    queryFn: async ({ signal }) => {
      const res = await gqlRequest<{ templates: Template[] }, { teamId: string | null }>(
        `query SettingsTemplates($teamId: String) { templates(teamId: $teamId) { ${TEMPLATE_FIELDS} } }`,
        { teamId },
        { signal },
      );
      return teamId ? res.templates : res.templates.filter((t) => !t.teamId);
    },
  });
}

export function useTemplateDefaults(teamId: string | undefined) {
  return useQuery({
    queryKey: templateKeys.defaults(teamId ?? ""),
    enabled: !!teamId,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ teamDefaultTemplates: TemplateDefaults }, { teamId: string }>(
          `query SettingsTemplateDefaults($teamId: String!) {
            teamDefaultTemplates(teamId: $teamId) { forMembersId forNonMembersId projectId }
          }`,
          { teamId: teamId! },
          { signal },
        )
      ).teamDefaultTemplates,
  });
}

export interface TemplateInput {
  type: TemplateKind;
  name: string;
  description?: string;
  teamId?: string;
  templateData: Record<string, unknown>;
}

export function useTemplateMutations() {
  const qc = useQueryClient();
  const done = () => {
    void qc.invalidateQueries({ queryKey: ["settings"] });
  };
  return {
    create: useMutation({
      mutationFn: async (input: TemplateInput) =>
        gqlRequest(
          `mutation SettingsTemplateCreate($input: TemplateCreateInput!) {
            templateCreate(input: $input) { success }
          }`,
          { input },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: async (v: { id: string; input: Record<string, unknown> }) =>
        gqlRequest(
          `mutation SettingsTemplateUpdate($id: String!, $input: TemplateUpdateInput!) {
            templateUpdate(id: $id, input: $input) { success }
          }`,
          v,
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: async (id: string) =>
        gqlRequest(
          `mutation SettingsTemplateDelete($id: String!) { templateDelete(id: $id) { success } }`,
          { id },
        ),
      onSuccess: done,
    }),
    setDefault: useMutation({
      mutationFn: async (v: { teamId: string; slot: DefaultSlot; templateId: string | null }) =>
        gqlRequest(
          `mutation SettingsTemplateDefaultSet($teamId: String!, $slot: TemplateDefaultSlot!, $templateId: String) {
            teamDefaultTemplateSet(teamId: $teamId, slot: $slot, templateId: $templateId)
          }`,
          v,
        ),
      onSuccess: done,
    }),
  };
}

// ---------- Workspace-Labels ----------

/** Alle Workspace-Labels (ohne Team) inklusive Gruppen und zurückgezogener Labels. */
export function useWorkspaceLabels() {
  return useQuery({
    queryKey: templateKeys.workspaceLabels,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ issueLabels: IssueLabel[] }>(
          `query SettingsWorkspaceLabels {
            issueLabels(includeGroups: true, first: 250) {
              id name color description teamId parentId isGroup groupType retiredAt
            }
          }`,
          undefined,
          { signal },
        )
      ).issueLabels.filter((l) => !l.teamId),
  });
}

export function useLabelRestore() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(
        `mutation SettingsLabelRestore($id: String!) { issueLabelRestore(id: $id) { success } }`,
        { id },
      ),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["settings"] });
      void qc.invalidateQueries({ queryKey: ["team-meta"] });
    },
  });
}
