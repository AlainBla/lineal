/** Testhilfen: erfundene Daten (acme, ENG, Alice/Bob) und ein GraphQL-Fetch-Mock nach Operationsnamen. */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import { vi } from "vitest";

import { SESSION_QUERY_KEY, type Session } from "@/app/auth";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { Issue, WorkflowState } from "@/gql/graphql";

import type { TeamMeta } from "./queries";

export const SESSION: Session = {
  user: { id: "user-bob", name: "Bob", email: "bob@acme.example", avatarUrl: null },
  workspace: { id: "org-1", name: "Acme", urlKey: "acme" },
  teams: [
    {
      id: "team-eng",
      key: "ENG",
      name: "Engineering",
      parentId: null,
      cyclesEnabled: true,
      triageEnabled: false,
    },
    {
      id: "team-des",
      key: "DES",
      name: "Design",
      parentId: null,
      cyclesEnabled: false,
      triageEnabled: false,
    },
  ],
};

export const STATES: WorkflowState[] = [
  st("s-backlog", "Backlog", "backlog", 0),
  st("s-todo", "Todo", "unstarted", 1),
  st("s-progress", "In Progress", "started", 2),
  st("s-done", "Done", "completed", 3),
  st("s-canceled", "Canceled", "canceled", 4),
];

function st(id: string, name: string, type: string, position: number): WorkflowState {
  return {
    id,
    teamId: "team-eng",
    name,
    type,
    color: "#5e6ad2",
    description: null,
    position,
    inheritedFromId: null,
    isDefault: id === "s-backlog",
  };
}

export function makeIssue(over: Partial<Issue> & Pick<Issue, "id" | "identifier">): Issue {
  const state = STATES.find((s) => s.id === (over.stateId ?? "s-todo")) ?? STATES[1]!;
  return {
    // Nachladefelder der Detailansicht (nicht Teil der Listenabfragen).
    children: [],
    relations: { blocks: [], blockedBy: [], relatedTo: [], duplicateOf: null, duplicates: [] },
    history: [],
    subscribed: false,
    previousIdentifiers: [],
    title: `Issue ${over.identifier}`,
    description: null,
    priority: 0,
    estimate: null,
    teamId: "team-eng",
    stateId: state.id,
    stateName: state.name,
    stateType: state.type,
    assigneeId: null,
    creatorId: "user-bob",
    parentId: null,
    projectId: null,
    projectMilestoneId: null,
    cycleId: null,
    labelIds: [],
    dueDate: null,
    sortOrder: 0,
    startedAt: null,
    completedAt: null,
    canceledAt: null,
    archivedAt: null,
    trashed: false,
    createdAt: "2026-10-01T10:00:00Z",
    updatedAt: "2026-10-02T10:00:00Z",
    stateEnteredAt: "2026-10-01T10:00:00Z",
    snoozedUntilAt: null,
    startedTriageAt: null,
    triagedAt: null,
    linkCount: 0,
    attachments: [],
    ...over,
  };
}

export const TEAM_META: TeamMeta = {
  team: {
    id: "team-eng",
    key: "ENG",
    name: "Engineering",
    displayName: "Engineering",
    parentId: null,
    description: null,
    icon: null,
    color: null,
    visibility: "public",
    allMembersCanJoin: true,
    memberCount: 2,
    viewer: { isMember: true, isOwner: false, canJoin: false },
    timezone: "UTC",
    triageEnabled: false,
    cyclesEnabled: true,
    cycleDuration: 2,
    cycleCooldownTime: 0,
    cycleStartDay: 1,
    upcomingCycleCount: 2,
    cycleIssueAutoAssignStarted: false,
    cycleIssueAutoAssignCompleted: false,
    cycleLockToActive: false,
    issueEstimationType: "fibonacci",
    inheritWorkflowStatuses: false,
    issueEstimationExtended: false,
    issueEstimationAllowZero: false,
    defaultIssueEstimate: 0,
    inheritIssueEstimation: false,
    autoClosePeriod: null,
    autoCloseStateId: null,
    autoArchivePeriod: 6,
    autoCloseParentIssues: false,
    autoCloseChildIssues: false,
    retiredAt: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  },
  states: STATES,
  labels: [
    {
      id: "l-bug",
      name: "Bug",
      color: "#eb5757",
      description: null,
      teamId: "team-eng",
      parentId: null,
      isGroup: false,
      groupType: null,
      retiredAt: null,
      archivedAt: null,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
    },
  ],
  cycles: [],
  projects: [
    {
      id: "p-1",
      identifier: "P-ENG-1",
      name: "Checkout Relaunch",
      color: "#4cb782",
      milestones: [
        { id: "m-alpha", name: "Alpha", targetDate: "2026-11-30", sortOrder: 1 },
        { id: "m-beta", name: "Beta", targetDate: "2027-01-31", sortOrder: 2 },
      ],
    },
  ],
};

export interface RecordedCall {
  operation: string;
  variables: Record<string, unknown>;
}

type Handler = (variables: Record<string, unknown>) => unknown;

/**
 * Ersetzt `fetch`: ruft pro GraphQL-Operationsname (`query TeamMeta`, `mutation IssueUpdate`) den Handler
 * auf und antwortet mit `{ data }`; ein Fehler-Handler kann `new Error("…")` zurückgeben.
 */
export function mockGraphql(handlers: Record<string, Handler>) {
  const calls: RecordedCall[] = [];
  const fn = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? "{}")) as {
      query: string;
      variables?: Record<string, unknown>;
    };
    const op = /(?:query|mutation)\s+(\w+)/.exec(body.query)?.[1] ?? "";
    const variables = body.variables ?? {};
    calls.push({ operation: op, variables });
    const handler = handlers[op];
    if (!handler) {
      return new Response(JSON.stringify({ errors: [{ message: `no handler for ${op}` }] }), {
        status: 200,
      });
    }
    const result = handler(variables);
    if (result instanceof Error) {
      const code = (result as Error & { code?: string }).code;
      return new Response(
        JSON.stringify({
          errors: [{ message: result.message, ...(code ? { extensions: { code } } : {}) }],
        }),
        { status: 200 },
      );
    }
    return new Response(JSON.stringify({ data: result }), { status: 200 });
  });
  vi.stubGlobal("fetch", fn);
  return { calls, fn, of: (op: string) => calls.filter((c) => c.operation === op) };
}

export function newClient(): QueryClient {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 30_000 }, mutations: { retry: false } },
  });
  client.setQueryData(SESSION_QUERY_KEY, SESSION);
  return client;
}

/** Rendert `ui` in Query-Client, Tooltip-Provider und Memory-Router (für `Link`, `useRouterState`). */
export async function renderWithApp(
  ui: ReactNode,
  client = newClient(),
  path = "/acme/team/ENG/all",
) {
  const rootRoute = createRootRoute({ component: () => <>{ui}</> });
  const router = createRouter({
    routeTree: rootRoute,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  await router.load();
  const utils = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { ...utils, client, router };
}
