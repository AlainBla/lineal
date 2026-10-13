/**
 * Daten der Kommentare an Dokumenten, Projekten, Initiativen und deren Status-Updates (F.06).
 * Ein Ziel (`CommentTarget`) benennt genau ein Elternobjekt; Issue-Kommentare haben ihre eigene
 * Oberfläche (features/issues/comments.tsx).
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commentAuthors, useRememberUsers } from "@/features/issues/users";
import type { Comment, PageInfo } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export type CommentTarget =
  | { documentId: string }
  | { projectId: string }
  | { initiativeId: string }
  | { projectUpdateId: string }
  | { initiativeUpdateId: string };

export const COMMENT_FIELDS = `id body userId user { id name displayName } documentId projectId initiativeId
  projectUpdateId initiativeUpdateId parentId quotedText anchorStart anchorEnd anchorState editedAt
  resolvedAt resolvingUserId createdAt updatedAt`;

export const commentKeys = {
  all: ["target-comments"] as const,
  list: (t: CommentTarget) => ["target-comments", JSON.stringify(t)] as const,
};

const PAGE = 100;
const MAX_PAGES = 20;

async function fetchTargetComments(t: CommentTarget, signal?: AbortSignal): Promise<Comment[]> {
  const [field, id] = Object.entries(t)[0] as [string, string];
  const all: Comment[] = [];
  let after: string | null = null;
  for (let page = 0; page < MAX_PAGES; page++) {
    const res: { comments: { nodes: Comment[]; pageInfo: PageInfo } } = await gqlRequest<
      { comments: { nodes: Comment[]; pageInfo: PageInfo } },
      Record<string, unknown>
    >(
      `query TargetComments($id: String, $first: Int, $after: String) {
        comments(${field}: $id, first: $first, after: $after, orderBy: createdAt) {
          nodes { ${COMMENT_FIELDS} } pageInfo { hasNextPage endCursor }
        }
      }`,
      { id, first: PAGE, after },
      { signal },
    );
    all.push(...res.comments.nodes);
    const info = res.comments.pageInfo;
    if (!info?.hasNextPage || !info.endCursor) break;
    after = info.endCursor;
  }
  return all;
}

export function useTargetComments(target: CommentTarget, refetchMs?: number) {
  const remember = useRememberUsers();
  return useQuery({
    queryKey: commentKeys.list(target),
    queryFn: async ({ signal }) => {
      const nodes = await fetchTargetComments(target, signal);
      remember(commentAuthors(nodes));
      return nodes;
    },
    refetchInterval: refetchMs,
  });
}

export interface InlineAnchor {
  quotedText: string;
  anchorStart: string;
  anchorEnd: string;
}

async function run(query: string, variables: Record<string, unknown>): Promise<void> {
  const res = await gqlRequest<Record<string, { success: boolean }>, Record<string, unknown>>(
    query,
    variables,
  );
  const payload = Object.values(res)[0];
  if (!payload?.success) throw new Error("Request failed");
}

export function useCommentMutations(target: CommentTarget) {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: commentKeys.all });
  return {
    create: useMutation({
      mutationFn: (v: { body: string; parentId?: string } & Partial<InlineAnchor>) =>
        run(
          `mutation TargetCommentCreate($input: CommentCreateInput!) {
            commentCreate(input: $input) { success }
          }`,
          { input: { ...target, ...v } },
        ),
      onSuccess: done,
    }),
    update: useMutation({
      mutationFn: (v: { id: string; body: string }) =>
        run(
          `mutation TargetCommentUpdate($id: String!, $input: CommentUpdateInput!) {
            commentUpdate(id: $id, input: $input) { success }
          }`,
          { id: v.id, input: { body: v.body } },
        ),
      onSuccess: done,
    }),
    remove: useMutation({
      mutationFn: (id: string) =>
        run(`mutation TargetCommentDelete($id: String!) { commentDelete(id: $id) { success } }`, {
          id,
        }),
      onSuccess: done,
    }),
    resolve: useMutation({
      mutationFn: (v: { id: string; resolved: boolean }) =>
        v.resolved
          ? run(
              `mutation TargetCommentResolve($id: String!) { commentResolve(id: $id) { success } }`,
              { id: v.id },
            )
          : run(
              `mutation TargetCommentUnresolve($id: String!) { commentUnresolve(id: $id) { success } }`,
              { id: v.id },
            ),
      onSuccess: done,
    }),
  };
}
