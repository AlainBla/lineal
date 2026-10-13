/**
 * GraphQL-Operationen und Query-Keys der Issue-Screens. Gegen `web/schema.graphql` programmiert:
 * Listen sind einfache Arrays bzw. `{ nodes, hasNextPage, endCursor }` (keine Relay-Connections).
 */
import type {
  Comment,
  Cycle,
  Issue,
  IssueConnection,
  IssueLabel,
  PageInfo,
  Project,
  Team,
  WorkflowState,
} from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const ISSUE_FIELDS = `
  id identifier previousIdentifiers title description priority estimate teamId
  stateId stateName stateType assigneeId creatorId parentId projectId projectMilestoneId cycleId
  labelIds dueDate sortOrder startedAt completedAt canceledAt archivedAt trashed
  slaType slaStartedAt slaBreachesAt createdAt updatedAt
  stateEnteredAt snoozedUntilAt startedTriageAt linkCount
`;

const COMMENT_FIELDS = `
  id body userId user { id name displayName } issueId parentId quotedText editedAt resolvedAt
  reactionData { emoji count userIds } createdAt updatedAt
`;

export const keys = {
  issues: ["issues"] as const,
  teamIssues: (team: string) => ["issues", "team", team] as const,
  myIssues: (tab: string, userId: string | undefined) => ["issues", "my", tab, userId] as const,
  issue: (ref: string) => ["issue", ref] as const,
  /** Nachladefelder der Detailansicht (children, relations, history); eigener Präfix, damit Listen-Patches sie nicht treffen. */
  issueDetail: (id: string) => ["issue-detail", id] as const,
  issueDetails: ["issue-detail"] as const,
  teamMeta: (team: string) => ["team-meta", team] as const,
  comments: (issueId: string) => ["comments", issueId] as const,
  teams: ["issue-teams"] as const,
};

/** Obergrenze geladener Issues je Liste (Seiten à 250); darüber weist die Ansicht darauf hin. */
export const PAGE_SIZE = 250;
export const MAX_PAGES = 8;

export interface IssueList {
  issues: Issue[];
  /** Es gibt mehr Issues als geladen (Obergrenze erreicht). */
  truncated: boolean;
}

interface IssuesVars extends Record<string, unknown> {
  teamId?: string;
  assigneeId?: string;
  first: number;
  after?: string;
}

const ISSUES_QUERY = `query TeamIssues($teamId: String, $assigneeId: String, $first: Int, $after: String) {
  issues(teamId: $teamId, assigneeId: $assigneeId, first: $first, after: $after) {
    nodes { ${ISSUE_FIELDS} } hasNextPage endCursor
  }
}`;

/** Lädt Issues seitenweise bis zur Obergrenze; Triage und Papierkorb werden ausgeblendet. */
export async function fetchIssues(
  filter: { teamId?: string; assigneeId?: string },
  signal?: AbortSignal,
): Promise<IssueList> {
  const all: Issue[] = [];
  let after: string | undefined;
  let truncated = false;
  for (let page = 0; page < MAX_PAGES; page++) {
    const vars: IssuesVars = { ...filter, first: PAGE_SIZE, after };
    const res = await gqlRequest<{ issues: IssueConnection }, IssuesVars>(ISSUES_QUERY, vars, {
      signal,
    });
    all.push(...res.issues.nodes);
    if (!res.issues.hasNextPage || !res.issues.endCursor) break;
    after = res.issues.endCursor;
    if (page === MAX_PAGES - 1) truncated = true;
  }
  return { issues: all.filter((i) => !i.trashed && i.stateType !== "triage"), truncated };
}

export interface TeamMeta {
  team: Team;
  states: WorkflowState[];
  labels: IssueLabel[];
  cycles: Cycle[];
  projects: Array<Pick<Project, "id" | "identifier" | "name" | "color">>;
}

const TEAM_META_QUERY = `query TeamMeta($team: String!) {
  team(id: $team) { id key name displayName parentId icon color issueEstimationType cyclesEnabled triageEnabled }
  workflowStates(teamId: $team) { id teamId name type color position isDefault }
  issueLabels(teamId: $team, includeGroups: true) { id name color teamId parentId isGroup groupType }
  cycles(teamId: $team) { id teamId number name startsAt endsAt isActive isNext isPast }
  projects(team: $team) { id identifier name color }
}`;

export async function fetchTeamMeta(team: string, signal?: AbortSignal): Promise<TeamMeta> {
  const res = await gqlRequest<
    {
      team: Team;
      workflowStates: WorkflowState[];
      issueLabels: IssueLabel[];
      cycles: Cycle[];
      projects: TeamMeta["projects"];
    },
    { team: string }
  >(TEAM_META_QUERY, { team }, { signal });
  return {
    team: res.team,
    states: res.workflowStates,
    labels: res.issueLabels,
    cycles: res.cycles,
    projects: res.projects,
  };
}

export async function fetchIssue(ref: string, signal?: AbortSignal): Promise<Issue> {
  const res = await gqlRequest<{ issue: Issue }, { id: string }>(
    `query IssueDetail($id: String!) { issue(id: $id) { ${ISSUE_FIELDS} } }`,
    { id: ref },
    { signal },
  );
  return res.issue;
}

/** Nachladefelder der Detailansicht; je Request höchstens 10 solcher Felder (hier 5). */
export type IssueExtras = Pick<
  Issue,
  "children" | "relations" | "history" | "subscribed" | "attachments"
>;

const RELATED = "id identifier title";
const EXTRAS_QUERY = `query IssueExtras($id: String!) {
  issue(id: $id) {
    id
    subscribed
    children(first: 100) { ${ISSUE_FIELDS} }
    relations {
      blocks { ${RELATED} } blockedBy { ${RELATED} } relatedTo { ${RELATED} }
      duplicateOf { ${RELATED} } duplicates { ${RELATED} }
    }
    history { id actorId actorName source changes addedLabelIds removedLabelIds relationChanges flags createdAt }
    attachments { id issueId title subtitle url sourceType contentType size creatorId createdAt }
  }
}`;

export async function fetchIssueExtras(id: string, signal?: AbortSignal): Promise<IssueExtras> {
  const res = await gqlRequest<{ issue: IssueExtras }, { id: string }>(
    EXTRAS_QUERY,
    { id },
    { signal },
  );
  return res.issue;
}

const COMMENTS_PAGE = 100;
const COMMENTS_MAX_PAGES = 50;

/** Alle Kommentare eines Issues, seitenweise nachgeladen (B4.14: nicht nur die ersten 100). */
export async function fetchComments(issueId: string, signal?: AbortSignal): Promise<Comment[]> {
  const all: Comment[] = [];
  let after: string | null = null;
  for (let page = 0; page < COMMENTS_MAX_PAGES; page++) {
    const res: { comments: { nodes: Comment[]; pageInfo: PageInfo } } = await gqlRequest<
      { comments: { nodes: Comment[]; pageInfo: PageInfo } },
      { issueId: string; first: number; after: string | null }
    >(
      `query IssueComments($issueId: String, $first: Int, $after: String) {
      comments(issueId: $issueId, first: $first, after: $after, orderBy: createdAt) {
        nodes { ${COMMENT_FIELDS} } pageInfo { hasNextPage endCursor }
      }
    }`,
      { issueId, first: COMMENTS_PAGE, after },
      { signal },
    );
    all.push(...res.comments.nodes);
    const info = res.comments.pageInfo;
    if (!info?.hasNextPage || !info.endCursor) break;
    after = info.endCursor;
  }
  return all;
}

/** Eingabe von `issueUpdate`; `null` leert ein Feld (Tri-State), ein fehlender Schlüssel lässt es unverändert. */
export interface IssueUpdateVars {
  [key: string]: unknown;
  title?: string;
  description?: string | null;
  stateId?: string;
  assigneeId?: string | null;
  priority?: number;
  estimate?: number | null;
  teamId?: string;
  parentId?: string | null;
  labelIds?: string[];
  projectId?: string | null;
  cycleId?: string | null;
  dueDate?: string | null;
  sortOrder?: number;
}

export async function updateIssue(id: string, input: IssueUpdateVars): Promise<Issue> {
  const res = await gqlRequest<
    { issueUpdate: { success: boolean; issue: Issue | null } },
    { id: string; input: IssueUpdateVars }
  >(
    `mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) {
      issueUpdate(id: $id, input: $input) { success issue { ${ISSUE_FIELDS} } }
    }`,
    { id, input },
  );
  if (!res.issueUpdate.success || !res.issueUpdate.issue) throw new Error("Update failed");
  return res.issueUpdate.issue;
}

/** Eingabe von `issueBatchUpdate` (ids plus dieselbe Änderung für alle). */
export interface IssueBatchVars {
  [key: string]: unknown;
  ids: string[];
  stateId?: string;
  priority?: number;
  assigneeId?: string | null;
  addedLabelIds?: string[];
  removedLabelIds?: string[];
  projectId?: string | null;
  cycleId?: string | null;
  estimate?: number | null;
  teamId?: string;
  moveSubIssues?: boolean;
  archive?: boolean;
  delete?: boolean;
}

export interface IssueBatchResult {
  issues: Issue[];
  failures: Array<{ id: string; code: string; message: string }>;
}

export async function batchUpdateIssues(input: IssueBatchVars): Promise<IssueBatchResult> {
  const res = await gqlRequest<{ issueBatchUpdate: IssueBatchResult }, { input: IssueBatchVars }>(
    `mutation IssueBatchUpdate($input: IssueBatchUpdateInput!) {
      issueBatchUpdate(input: $input) {
        issues { ${ISSUE_FIELDS} } failures { id code message }
      }
    }`,
    { input },
  );
  return res.issueBatchUpdate;
}

export interface IssueCreateVars {
  [key: string]: unknown;
  teamId: string;
  /** Pflicht, außer ein Template liefert den Titel. */
  title?: string;
  description?: string;
  stateId?: string;
  assigneeId?: string;
  priority?: number;
  estimate?: number;
  labelIds?: string[];
  parentId?: string;
  projectId?: string;
  cycleId?: string;
  dueDate?: string;
  /** Template (UUID); der Server wendet es an (Merge, Platzhalter, Formular). */
  templateId?: string;
  formAnswers?: Array<{ fieldId: string; value: string | string[] | number }>;
}

export async function createIssue(input: IssueCreateVars): Promise<Issue> {
  const res = await gqlRequest<
    { issueCreate: { success: boolean; issue: Issue | null } },
    { input: IssueCreateVars }
  >(
    `mutation IssueCreate($input: IssueCreateInput!) {
      issueCreate(input: $input) { success issue { ${ISSUE_FIELDS} } }
    }`,
    { input },
  );
  if (!res.issueCreate.success || !res.issueCreate.issue) throw new Error("Create failed");
  return res.issueCreate.issue;
}

export async function createComment(input: {
  issueId: string;
  body: string;
  parentId?: string;
}): Promise<Comment> {
  const res = await gqlRequest<
    { commentCreate: { success: boolean; comment: Comment | null } },
    { input: typeof input }
  >(
    `mutation CommentCreate($input: CommentCreateInput!) {
      commentCreate(input: $input) { success comment { ${COMMENT_FIELDS} } }
    }`,
    { input },
  );
  if (!res.commentCreate.success || !res.commentCreate.comment) throw new Error("Comment failed");
  return res.commentCreate.comment;
}

export async function updateComment(id: string, body: string): Promise<Comment> {
  const res = await gqlRequest<
    { commentUpdate: { success: boolean; comment: Comment | null } },
    { id: string; input: { body: string } }
  >(
    `mutation CommentUpdate($id: String!, $input: CommentUpdateInput!) {
      commentUpdate(id: $id, input: $input) { success comment { ${COMMENT_FIELDS} } }
    }`,
    { id, input: { body } },
  );
  if (!res.commentUpdate.success || !res.commentUpdate.comment) throw new Error("Update failed");
  return res.commentUpdate.comment;
}

export async function deleteComment(id: string): Promise<void> {
  await gqlRequest<{ commentDelete: { success: boolean } }, { id: string }>(
    `mutation CommentDelete($id: String!) { commentDelete(id: $id) { success } }`,
    { id },
  );
}

/** Thread auflösen (`resolve = true`) oder wiedereröffnen. */
export async function setCommentResolved(id: string, resolve: boolean): Promise<Comment> {
  const op = resolve ? "commentResolve" : "commentUnresolve";
  const res = await gqlRequest<
    Record<string, { success: boolean; comment: Comment | null }>,
    { id: string }
  >(
    `mutation CommentResolution($id: String!) {
      ${op}(id: $id) { success comment { ${COMMENT_FIELDS} } }
    }`,
    { id },
  );
  const comment = res[op]?.comment;
  if (!comment) throw new Error("Update failed");
  return comment;
}

/** Hängt eine hochgeladene Datei (`assetUrl` aus `fileUpload`) als Anhang an das Issue. */
export async function createAttachment(input: {
  issueId: string;
  url: string;
  title?: string;
}): Promise<void> {
  await gqlRequest<{ attachmentCreate: { success: boolean } }, { input: typeof input }>(
    `mutation AttachmentCreate($input: AttachmentCreateInput!) {
      attachmentCreate(input: $input) { success }
    }`,
    { input },
  );
}

export async function deleteAttachment(id: string): Promise<void> {
  await gqlRequest<{ attachmentDelete: { success: boolean } }, { id: string }>(
    `mutation AttachmentDelete($id: String!) { attachmentDelete(id: $id) { success } }`,
    { id },
  );
}

export async function createReaction(
  commentId: string,
  emoji: string,
): Promise<{ id: string; emoji: string }> {
  const res = await gqlRequest<
    { reactionCreate: { success: boolean; reaction: { id: string; emoji: string } | null } },
    { input: { commentId: string; emoji: string } }
  >(
    `mutation ReactionCreate($input: ReactionCreateInput!) {
      reactionCreate(input: $input) { success reaction { id emoji } }
    }`,
    { input: { commentId, emoji } },
  );
  if (!res.reactionCreate.reaction) throw new Error("Reaction failed");
  return res.reactionCreate.reaction;
}

export async function deleteReaction(id: string): Promise<void> {
  await gqlRequest<{ reactionDelete: { success: boolean } }, { id: string }>(
    `mutation ReactionDelete($id: String!) { reactionDelete(id: $id) { success } }`,
    { id },
  );
}

export type RelationKind = "blocks" | "related" | "duplicate";

export async function createRelation(input: {
  issueId: string;
  relatedIssueId: string;
  type: RelationKind;
}): Promise<void> {
  await gqlRequest<
    { issueRelationCreate: { success: boolean } },
    { input: { issueId: string; relatedIssueId: string; type: RelationKind } }
  >(
    `mutation IssueRelationCreate($input: IssueRelationCreateInput!) {
      issueRelationCreate(input: $input) { success }
    }`,
    { input },
  );
}

export async function setSubscription(issueId: string, subscribe: boolean): Promise<void> {
  const op = subscribe ? "issueSubscribe" : "issueUnsubscribe";
  await gqlRequest<Record<string, { success: boolean }>, { id: string }>(
    `mutation IssueSubscription($id: String!) { ${op}(id: $id) { success changed } }`,
    { id: issueId },
  );
}

export type TeamRef = Pick<Team, "id" | "key" | "name" | "parentId" | "issueEstimationType">;

/** Alle für den User sichtbaren, nicht archivierten Teams (Team-Picker, Create-Modal). */
export async function fetchTeams(signal?: AbortSignal): Promise<TeamRef[]> {
  const res = await gqlRequest<{ teams: TeamRef[] }>(
    `query IssueTeams { teams { id key name parentId issueEstimationType } }`,
    undefined,
    { signal },
  );
  return res.teams;
}
