/** TanStack-Query-Hooks der Issue-Screens inkl. optimistischer Updates mit Rollback. */
import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryKey,
} from "@tanstack/react-query";

import { useSession } from "@/app/auth";
import type { Issue } from "@/gql/graphql";

import type { IssuePatch } from "./model";
import {
  createComment,
  createIssue,
  createReaction,
  deleteReaction,
  fetchComments,
  fetchIssue,
  fetchIssueExtras,
  fetchIssues,
  fetchTeamMeta,
  fetchTeams,
  keys,
  updateIssue,
  type IssueCreateVars,
  type IssueList,
  type IssueUpdateVars,
} from "./queries";
import { commentAuthors, useRememberUsers } from "./users";

export function useTeamIssues(teamRef: string) {
  return useQuery({
    queryKey: keys.teamIssues(teamRef),
    queryFn: ({ signal }) => fetchIssues({ teamId: teamRef }, signal),
  });
}

export function useMyAssigned() {
  const session = useSession();
  return useQuery({
    queryKey: keys.myIssues("assigned", session?.user.id),
    enabled: !!session,
    queryFn: ({ signal }) => fetchIssues({ assigneeId: "me" }, signal),
  });
}

/** „Created“ hat im Schema keinen Filter: alle sichtbaren Issues laden, nach `creatorId` einschränken. */
export function useMyCreated() {
  const session = useSession();
  return useQuery({
    queryKey: keys.myIssues("created", session?.user.id),
    enabled: !!session,
    queryFn: async ({ signal }): Promise<IssueList> => {
      const list = await fetchIssues({}, signal);
      return { ...list, issues: list.issues.filter((i) => i.creatorId === session?.user.id) };
    },
  });
}

export function useTeamMeta(teamRef: string | undefined) {
  return useQuery({
    queryKey: keys.teamMeta(teamRef ?? ""),
    enabled: !!teamRef,
    queryFn: ({ signal }) => fetchTeamMeta(teamRef!, signal),
    staleTime: 60_000,
  });
}

/** Metadaten mehrerer Teams (My Issues); die Liste ist über Renderrunden strukturell stabil. */
export function useTeamMetas(refs: string[]) {
  return useQueries({
    queries: refs.map((r) => ({
      queryKey: keys.teamMeta(r),
      queryFn: ({ signal }: { signal: AbortSignal }) => fetchTeamMeta(r, signal),
      staleTime: 60_000,
    })),
    combine: (results) => results.map((r) => r.data),
  });
}

export function useTeams() {
  return useQuery({
    queryKey: keys.teams,
    queryFn: ({ signal }) => fetchTeams(signal),
    staleTime: 60_000,
  });
}

export function useIssue(ref: string) {
  return useQuery({
    queryKey: keys.issue(ref),
    enabled: ref !== "",
    queryFn: ({ signal }) => fetchIssue(ref, signal),
  });
}

/** Kinder, Relationen und Verlauf eines Issues (nur Detailansicht; nie in Listen). */
export function useIssueExtras(issueId: string | undefined) {
  return useQuery({
    queryKey: keys.issueDetail(issueId ?? ""),
    enabled: !!issueId,
    queryFn: ({ signal }) => fetchIssueExtras(issueId!, signal),
  });
}

export function useComments(issueId: string | undefined) {
  const remember = useRememberUsers();
  return useQuery({
    queryKey: keys.comments(issueId ?? ""),
    enabled: !!issueId,
    queryFn: async ({ signal }) => {
      const nodes = await fetchComments(issueId!, signal);
      remember(commentAuthors(nodes));
      return nodes;
    },
  });
}

type Snapshot = Array<[QueryKey, unknown]>;

/** Wendet `patch` auf jedes gecachte Vorkommen des Issues an (Listen und Detail). */
export function patchIssueCaches(qc: QueryClient, id: string, patch: IssuePatch): void {
  qc.setQueriesData<IssueList>({ queryKey: keys.issues }, (old) =>
    old && Array.isArray(old.issues)
      ? { ...old, issues: old.issues.map((i) => (i.id === id ? { ...i, ...patch } : i)) }
      : old,
  );
  qc.setQueriesData<Issue>({ queryKey: ["issue"] }, (old) =>
    old && old.id === id ? { ...old, ...patch } : old,
  );
}

export interface UpdateArgs {
  id: string;
  input: IssueUpdateVars;
  /** Optimistische Änderung am Cache; fehlt sie, wird nur gesendet. */
  patch?: IssuePatch;
}

/** `issueUpdate` mit optimistischem Cache-Update, Rollback bei Fehler und Abgleich mit der Serverantwort. */
export function useUpdateIssue() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: UpdateArgs) => updateIssue(id, input),
    onMutate: async ({ id, patch }) => {
      await Promise.all([
        qc.cancelQueries({ queryKey: keys.issues }),
        qc.cancelQueries({ queryKey: ["issue"] }),
      ]);
      const snapshot: Snapshot = [
        ...qc.getQueriesData({ queryKey: keys.issues }),
        ...qc.getQueriesData({ queryKey: ["issue"] }),
      ];
      if (patch) patchIssueCaches(qc, id, patch);
      return { snapshot };
    },
    onError: (_err, _vars, ctx) => {
      ctx?.snapshot.forEach(([key, data]) => qc.setQueryData(key, data));
    },
    onSuccess: (issue) => {
      patchIssueCaches(qc, issue.id, issue);
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: keys.issues });
      void qc.invalidateQueries({ queryKey: ["issue"] });
      void qc.invalidateQueries({ queryKey: keys.issueDetails });
    },
  });
}

export function useCreateIssue() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: IssueCreateVars) => createIssue(input),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: keys.issues });
      void qc.invalidateQueries({ queryKey: ["issue"] });
      void qc.invalidateQueries({ queryKey: keys.issueDetails });
    },
  });
}

export function useCreateComment(issueId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { body: string; parentId?: string }) => createComment({ issueId, ...args }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: keys.comments(issueId) }),
  });
}

export function useReactions(issueId: string) {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: keys.comments(issueId) });
  return {
    add: useMutation({
      mutationFn: (a: { commentId: string; emoji: string }) => createReaction(a.commentId, a.emoji),
      onSuccess: done,
    }),
    remove: useMutation({ mutationFn: (id: string) => deleteReaction(id), onSuccess: done }),
  };
}
