/** Dokumentlisten, Anlegen, Verschieben, Papierkorb und Abo (F.05). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { Document, DocumentConnection } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

import { DOC_FIELDS, docKeys } from "./queries";

/** Elternobjekt einer Dokumentliste bzw. eines neuen Dokuments (genau ein Elternschlüssel). */
export type DocScope =
  | { projectId: string }
  | { initiativeId: string }
  | { teamId: string }
  | { issueId: string }
  | { cycleId: string; teamId?: string };

const LIST_FIELDS = `id slugId title icon color url projectId issueId initiativeId cycleId teamId trashed updatedAt`;

export type DocListItem = Pick<
  Document,
  | "id"
  | "slugId"
  | "title"
  | "icon"
  | "color"
  | "url"
  | "projectId"
  | "issueId"
  | "initiativeId"
  | "cycleId"
  | "teamId"
  | "trashed"
  | "updatedAt"
>;

export const docListKeys = {
  list: (scope: DocScope, trashed: boolean) =>
    ["docs", "list", JSON.stringify(scope), trashed] as const,
  templates: (teamId: string | undefined) => ["docs", "templates", teamId ?? ""] as const,
  subscribed: (id: string) => ["docs", "subscribed", id] as const,
};

export function useDocuments(scope: DocScope, trashed = false) {
  return useQuery({
    queryKey: docListKeys.list(scope, trashed),
    queryFn: async ({ signal }): Promise<DocListItem[]> =>
      (
        await gqlRequest<{ documents: DocumentConnection }, Record<string, unknown>>(
          `query DocumentList($projectId: String, $initiativeId: String, $teamId: String, $issueId: String,
              $cycleId: String, $trashed: Boolean) {
            documents(projectId: $projectId, initiativeId: $initiativeId, teamId: $teamId, issueId: $issueId,
                cycleId: $cycleId, trashed: $trashed, first: 100) {
              nodes { ${LIST_FIELDS} }
            }
          }`,
          { ...scope, trashed },
          { signal },
        )
      ).documents.nodes,
  });
}

export interface DocTemplate {
  id: string;
  name: string;
}

/** Dokument-Templates, die für den Ort sichtbar sind (Team-, geerbte und Workspace-Templates). */
export function useDocumentTemplates(teamId: string | undefined) {
  return useQuery({
    queryKey: docListKeys.templates(teamId),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ templates: DocTemplate[] }, { teamId: string | null }>(
          `query DocumentTemplates($teamId: String) {
            templates(teamId: $teamId, type: document) { id name }
          }`,
          { teamId: teamId ?? null },
          { signal },
        )
      ).templates,
  });
}

function invalidateDocs(qc: ReturnType<typeof useQueryClient>) {
  void qc.invalidateQueries({ queryKey: ["docs"] });
  // Die Dokumentliste der Initiative (Planung) hat einen eigenen Schlüssel.
  void qc.invalidateQueries({ queryKey: ["planning", "initiative-documents"] });
}

/** Legt ein leeres Dokument oder eines aus einem Template an; `title` darf bei `templateId` fehlen. */
export function useCreateDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { scope: DocScope; title?: string; templateId?: string }) => {
      const res = await gqlRequest<
        { documentCreate: { success: boolean; document: Document | null } },
        { input: Record<string, unknown> }
      >(
        `mutation DocumentCreate($input: DocumentCreateInput!) {
          documentCreate(input: $input) { success document { ${DOC_FIELDS} } }
        }`,
        {
          input: {
            ...v.scope,
            ...(v.title ? { title: v.title } : {}),
            ...(v.templateId ? { templateId: v.templateId } : {}),
          },
        },
      );
      if (!res.documentCreate.document) throw new Error("Could not create the document");
      return res.documentCreate.document;
    },
    onSuccess: () => invalidateDocs(qc),
  });
}

/** Verschiebt das Dokument zu einem neuen Elternobjekt (Referenzen: Slug, Kennung, Teamkürzel). */
export function useMoveDocument(ref: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; parent: DocScope }) => {
      const res = await gqlRequest<
        { documentUpdate: { success: boolean; document: Document | null } },
        { id: string; input: DocScope }
      >(
        `mutation DocumentMove($id: String!, $input: DocumentUpdateInput!) {
          documentUpdate(id: $id, input: $input) { success document { ${DOC_FIELDS} } }
        }`,
        { id: v.id, input: v.parent },
      );
      if (!res.documentUpdate.document) throw new Error("Could not move the document");
      return res.documentUpdate.document;
    },
    onSuccess: (doc) => {
      qc.setQueryData(docKeys.doc(ref), doc);
      invalidateDocs(qc);
    },
  });
}

export function useSetDocumentIcon(ref: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; icon: string | null }) =>
      gqlRequest(
        `mutation DocumentIcon($id: String!, $input: DocumentUpdateInput!) {
          documentUpdate(id: $id, input: $input) { success }
        }`,
        { id: v.id, input: { icon: v.icon } },
      ),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: docKeys.doc(ref) });
      invalidateDocs(qc);
    },
  });
}

/** Löschen (Papierkorb) bzw. Wiederherstellen. */
export function useTrashDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; restore: boolean }) =>
      gqlRequest(
        v.restore
          ? `mutation DocumentRestoreTrash($id: String!) { documentUnarchive(id: $id) { success } }`
          : `mutation DocumentTrash($id: String!) { documentDelete(id: $id) { success } }`,
        { id: v.id },
      ),
    onSuccess: () => {
      invalidateDocs(qc);
      void qc.invalidateQueries({ queryKey: ["docs", "doc"] });
    },
  });
}

export function useDocumentSubscription(id: string | undefined) {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: docListKeys.subscribed(id ?? ""),
    enabled: !!id,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ documentSubscribed: boolean }, { id: string }>(
          `query DocumentSubscribed($id: String!) { documentSubscribed(id: $id) }`,
          { id: id! },
          { signal },
        )
      ).documentSubscribed,
  });
  const toggle = useMutation({
    mutationFn: async (subscribe: boolean) =>
      gqlRequest(
        subscribe
          ? `mutation DocumentSubscribe($id: String!) { documentSubscribe(id: $id) { success subscribed } }`
          : `mutation DocumentUnsubscribe($id: String!) { documentUnsubscribe(id: $id) { success subscribed } }`,
        { id },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docListKeys.subscribed(id ?? "") }),
  });
  return { subscribed: q.data ?? false, loading: q.isLoading, toggle };
}
