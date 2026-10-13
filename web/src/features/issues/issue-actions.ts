/** Feldänderungen an Issues als benannte Aktionen (optimistisch, mit Fehler-Toast). */
import { useMemo } from "react";

import { useSession } from "@/app/auth";
import type { Issue, WorkflowState } from "@/gql/graphql";

import { useUpdateIssue, type UpdateArgs } from "./hooks";
import { describeStatePatch } from "./model";

export interface IssueActions {
  setState: (issue: Issue, stateId: string, states: WorkflowState[]) => void;
  setPriority: (issue: Issue, priority: number) => void;
  setAssignee: (issue: Issue, assigneeId: string | null) => void;
  assignToMe: (issue: Issue) => void;
  setLabels: (issue: Issue, labelIds: string[]) => void;
  setEstimate: (issue: Issue, estimate: number | null) => void;
  setProject: (issue: Issue, projectId: string | null) => void;
  setMilestone: (issue: Issue, milestoneId: string | null) => void;
  setCycle: (issue: Issue, cycleId: string | null) => void;
  setDueDate: (issue: Issue, dueDate: string | null) => void;
  setTitle: (issue: Issue, title: string) => void;
  setDescription: (issue: Issue, description: string) => Promise<void>;
  setParent: (issue: Issue, parentId: string | null) => void;
  /** Teamwechsel; liefert das aktualisierte Issue (neue Kennung) oder `null` bei Fehler. */
  moveToTeam: (issue: Issue, teamId: string) => Promise<Issue | null>;
  setSortOrder: (
    issue: Issue,
    patch: { stateId?: string; sortOrder: number },
    states: WorkflowState[],
  ) => void;
}

export function useIssueActions(): IssueActions {
  const mutation = useUpdateIssue();
  const session = useSession();
  const { mutate, mutateAsync } = mutation;

  return useMemo(() => {
    // Fehler-Toast und Rollback übernimmt `useUpdateIssue` (gilt für jeden Aufruf).
    const send = (args: UpdateArgs) => mutate(args);
    return {
      setState: (i, stateId, states) =>
        send({ id: i.id, input: { stateId }, patch: describeStatePatch(states, stateId) }),
      setPriority: (i, priority) => send({ id: i.id, input: { priority }, patch: { priority } }),
      setAssignee: (i, assigneeId) =>
        send({ id: i.id, input: { assigneeId }, patch: { assigneeId } }),
      assignToMe: (i) => {
        if (!session) return;
        send({
          id: i.id,
          input: { assigneeId: session.user.id },
          patch: { assigneeId: session.user.id },
        });
      },
      setLabels: (i, labelIds) => send({ id: i.id, input: { labelIds }, patch: { labelIds } }),
      setEstimate: (i, estimate) => send({ id: i.id, input: { estimate }, patch: { estimate } }),
      // Ein Projektwechsel löst den Meilenstein (docs/04 §3.4); der Server liefert den Rest.
      setProject: (i, projectId) =>
        send({
          id: i.id,
          input: { projectId },
          patch: { projectId, projectMilestoneId: null },
        }),
      setMilestone: (i, projectMilestoneId) =>
        send({ id: i.id, input: { projectMilestoneId }, patch: { projectMilestoneId } }),
      setCycle: (i, cycleId) => send({ id: i.id, input: { cycleId }, patch: { cycleId } }),
      setDueDate: (i, dueDate) => send({ id: i.id, input: { dueDate }, patch: { dueDate } }),
      setTitle: (i, title) => send({ id: i.id, input: { title }, patch: { title } }),
      setDescription: async (i, description) => {
        await mutateAsync({ id: i.id, input: { description }, patch: { description } });
      },
      setParent: (i, parentId) => send({ id: i.id, input: { parentId }, patch: { parentId } }),
      moveToTeam: async (i, teamId) => {
        try {
          return await mutateAsync({ id: i.id, input: { teamId } });
        } catch {
          return null;
        }
      },
      setSortOrder: (i, p, states) =>
        send({
          id: i.id,
          input: { ...(p.stateId ? { stateId: p.stateId } : {}), sortOrder: p.sortOrder },
          patch: {
            sortOrder: p.sortOrder,
            ...(p.stateId ? describeStatePatch(states, p.stateId) : {}),
          },
        }),
    };
  }, [mutate, mutateAsync, session]);
}
