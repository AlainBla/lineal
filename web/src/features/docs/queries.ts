/** Daten der Dokumente: Abruf, Autosave mit Optimistic Locking, Historie und Wiederherstellen. */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  Document,
  DocumentContentHistory,
  DocumentContentHistoryConnection,
} from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const docKeys = {
  doc: (ref: string) => ["docs", "doc", ref] as const,
  history: (id: string) => ["docs", "history", id] as const,
};

const DOC_FIELDS = `id slugId title content icon color url projectId issueId initiativeId cycleId teamId
  contentVersion trashed createdAt updatedAt archivedAt warnings`;

export function useDocument(ref: string) {
  return useQuery({
    queryKey: docKeys.doc(ref),
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ document: Document }, { id: string }>(
          `query DocumentDetail($id: String!) { document(id: $id) { ${DOC_FIELDS} } }`,
          { id: ref },
          { signal },
        )
      ).document,
    // Titeländerungen anderer Nutzer erscheinen ohne Neuladen (B2.09).
    refetchInterval: 5000,
  });
}

export interface DocumentPatch {
  title?: string;
  content?: string;
}

/** `documentUpdate` mit `expectedVersion`; ein Versionskonflikt kommt als Fehler zurück (Aufrufer lädt neu). */
export function useSaveDocument(ref: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: string; expectedVersion?: number; patch: DocumentPatch }) => {
      const res = await gqlRequest<
        { documentUpdate: { success: boolean; document: Document | null } },
        { id: string; input: Record<string, unknown> }
      >(
        `mutation DocumentSave($id: String!, $input: DocumentUpdateInput!) {
          documentUpdate(id: $id, input: $input) { success document { ${DOC_FIELDS} } }
        }`,
        {
          id: v.id,
          input: {
            ...v.patch,
            ...(v.expectedVersion === undefined ? {} : { expectedVersion: v.expectedVersion }),
          },
        },
      );
      if (!res.documentUpdate.document) throw new Error("Save failed");
      return res.documentUpdate.document;
    },
    onSuccess: (doc) => {
      qc.setQueryData(docKeys.doc(ref), doc);
      void qc.invalidateQueries({ queryKey: ["docs", "history", doc.id] });
    },
  });
}

export function useDocumentHistory(documentId: string | undefined, enabled: boolean) {
  return useQuery({
    queryKey: docKeys.history(documentId ?? ""),
    enabled: enabled && !!documentId,
    queryFn: async ({ signal }): Promise<DocumentContentHistory[]> =>
      (
        await gqlRequest<
          { documentContentHistory: DocumentContentHistoryConnection },
          { documentId: string }
        >(
          `query DocumentHistory($documentId: String!) {
            documentContentHistory(documentId: $documentId, first: 50) {
              nodes { id markdown title actorIds kind restoredFromId contentSnapshotAt }
            }
          }`,
          { documentId: documentId! },
          { signal },
        )
      ).documentContentHistory.nodes,
  });
}

export function useRestoreDocument(ref: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { documentId: string; historyId: string }) =>
      gqlRequest(
        `mutation DocumentRestore($documentId: String!, $historyId: String!) {
          documentContentRestore(documentId: $documentId, historyId: $historyId) { success }
        }`,
        v,
      ),
    onSuccess: () =>
      void qc
        .invalidateQueries({ queryKey: ["docs"] })
        .then(() => qc.invalidateQueries({ queryKey: docKeys.doc(ref) })),
  });
}
