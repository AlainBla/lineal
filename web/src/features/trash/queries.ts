/**
 * „Recently deleted“ (F.04): Papierkorb-Listen für Issues, Projekte und Initiativen samt Restore.
 * Sichtbarkeit und Rechte entscheidet der Server (Teamsicht, Aufbewahrungsfrist); der Client filtert nichts.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ISSUE_FIELDS } from "@/features/issues/queries";
import type { Initiative, Issue, Project } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const trashKeys = {
  issues: (team: string) => ["trash", "issues", team] as const,
  projects: ["trash", "projects"] as const,
  initiatives: ["trash", "initiatives"] as const,
};

export function useTrashedIssues(team?: string) {
  return useQuery({
    queryKey: trashKeys.issues(team ?? ""),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ trashedIssues: Issue[] }, { team?: string }>(
          `query TrashIssues($team: String) { trashedIssues(team: $team, first: 100) { ${ISSUE_FIELDS} } }`,
          { team },
          { signal },
        )
      ).trashedIssues,
  });
}

export function useTrashedProjects() {
  return useQuery({
    queryKey: trashKeys.projects,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ trashedProjects: Project[] }>(
          `query TrashProjects { trashedProjects(first: 100) { id identifier name updatedAt leadTeam { id key name } } }`,
          undefined,
          { signal },
        )
      ).trashedProjects,
  });
}

export function useTrashedInitiatives(enabled = true) {
  return useQuery({
    queryKey: trashKeys.initiatives,
    enabled,
    retry: false,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ trashedInitiatives: Initiative[] }>(
          `query TrashInitiatives { trashedInitiatives(first: 100) { id identifier name updatedAt } }`,
          undefined,
          { signal },
        )
      ).trashedInitiatives,
  });
}

/** Stellt ein Objekt wieder her; die Rechte prüft der Server. */
export function useRestore(kind: "issue" | "project" | "initiative") {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      if (kind === "issue")
        return gqlRequest(
          `mutation TrashRestoreIssue($id: String!) { issueUnarchive(id: $id) { success } }`,
          { id },
        );
      if (kind === "project")
        return gqlRequest(
          `mutation TrashRestoreProject($id: String!) { projectDelete(id: $id, restore: true) { success } }`,
          { id },
        );
      return gqlRequest(
        `mutation TrashRestoreInitiative($id: String!) { initiativeDelete(id: $id, restore: true) { success } }`,
        { id },
      );
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["trash"] });
      void qc.invalidateQueries({ queryKey: ["issues"] });
      void qc.invalidateQueries({ queryKey: ["planning"] });
      void qc.invalidateQueries({ queryKey: ["team-meta"] });
    },
  });
}
