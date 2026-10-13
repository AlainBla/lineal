/**
 * GraphQL-Operationen der Triage-Ansicht (docs/03 §10, docs/08 §2.5). Die Query-Keys liegen unter
 * dem Präfix `["issues"]`, damit jede Issue-Mutation Liste und Sidebar-Zähler neu lädt.
 */
import type { Issue } from "@/gql/graphql";
import { ISSUE_FIELDS } from "@/features/issues/queries";
import { gqlRequest } from "@/lib/graphql";

export const triageKeys = {
  list: (teamId: string, includeSnoozed: boolean) =>
    ["issues", "triage", teamId, includeSnoozed] as const,
  count: (teamId: string) => ["issues", "triage-count", teamId] as const,
};

/** Obergrenze der geladenen Triage-Issues (Liste und Zähler). */
export const TRIAGE_LIMIT = 250;

export async function fetchTriageIssues(
  teamId: string,
  includeSnoozed: boolean,
  signal?: AbortSignal,
): Promise<Issue[]> {
  const res = await gqlRequest<
    { triageIssues: Issue[] },
    { teamId: string; includeSnoozed: boolean; first: number }
  >(
    `query TriageIssues($teamId: String!, $includeSnoozed: Boolean, $first: Int) {
      triageIssues(teamId: $teamId, includeSnoozed: $includeSnoozed, first: $first) { ${ISSUE_FIELDS} }
    }`,
    { teamId, includeSnoozed, first: TRIAGE_LIMIT },
    { signal },
  );
  return res.triageIssues;
}

/** Anzahl nicht gesnoozter Triage-Issues (Sidebar-Badge). */
export async function fetchTriageCount(teamId: string, signal?: AbortSignal): Promise<number> {
  const res = await gqlRequest<{ triageIssues: Array<{ id: string }> }, Record<string, unknown>>(
    `query TriageCount($teamId: String!, $first: Int) {
      triageIssues(teamId: $teamId, first: $first) { id }
    }`,
    { teamId, first: TRIAGE_LIMIT },
    { signal },
  );
  return res.triageIssues.length;
}

export interface TriageAcceptVars {
  stateId?: string;
  priority?: number;
  comment?: string;
}

async function triageMutation(
  field: string,
  decl: string,
  args: string,
  variables: Record<string, unknown>,
): Promise<Issue> {
  const op = field.charAt(0).toUpperCase() + field.slice(1);
  const res = await gqlRequest<
    Record<string, { success: boolean; issue: Issue | null }>,
    Record<string, unknown>
  >(
    `mutation ${op}(${decl}) { ${field}(${args}) { success issue { ${ISSUE_FIELDS} } } }`,
    variables,
  );
  const issue = res[field]?.issue;
  if (!issue) throw new Error("Triage action failed");
  return issue;
}

export const triageAccept = (id: string, input: TriageAcceptVars) =>
  triageMutation(
    "issueTriageAccept",
    "$id: String!, $input: TriageAcceptGqlInput",
    "id: $id, input: $input",
    { id, input },
  );

export const triageDecline = (id: string, input: TriageAcceptVars) =>
  triageMutation(
    "issueTriageDecline",
    "$id: String!, $input: TriageDeclineGqlInput",
    "id: $id, input: $input",
    { id, input },
  );

export const triageDuplicate = (id: string, duplicateOfId: string, comment?: string) =>
  triageMutation(
    "issueTriageDuplicate",
    "$id: String!, $duplicateOfId: String!, $comment: String",
    "id: $id, duplicateOfId: $duplicateOfId, comment: $comment",
    { id, duplicateOfId, comment },
  );

export const triageSnooze = (id: string, snoozedUntilAt: string) =>
  triageMutation(
    "issueTriageSnooze",
    "$id: String!, $snoozedUntilAt: DateTime!",
    "id: $id, snoozedUntilAt: $snoozedUntilAt",
    { id, snoozedUntilAt },
  );

export const triageUnsnooze = (id: string) =>
  triageMutation("issueTriageUnsnooze", "$id: String!", "id: $id", { id });

export interface TriageResponsibility {
  triageEnabled: boolean;
  action: string | null;
  currentUser: { id: string; name: string } | null;
}

export async function fetchTriageResponsibility(
  teamId: string,
  signal?: AbortSignal,
): Promise<TriageResponsibility> {
  const res = await gqlRequest<{ triageResponsibility: TriageResponsibility }, { teamId: string }>(
    `query TriageResponsibility($teamId: String!) {
      triageResponsibility(teamId: $teamId) { triageEnabled action currentUser { id name } }
    }`,
    { teamId },
    { signal },
  );
  return res.triageResponsibility;
}
