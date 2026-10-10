import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { Providers } from "@/app/providers";
import { queryClient } from "@/app/query-client";
import { setUiState } from "@/app/ui-state";
import { shortcuts } from "@/lib/shortcuts";

import { routeTree } from "../../routeTree.gen";
import { makeIssue, mockGraphql, SESSION, STATES, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { resetKnownUsers } from "./users";

vi.mock("./rich-editor", () => ({
  RichEditor: (p: { ariaLabel: string; value: string }) => (
    <textarea aria-label={p.ariaLabel} defaultValue={p.value} />
  ),
}));

const ME = {
  user: { id: SESSION.user.id, displayName: "Bob", email: SESSION.user.email },
  organization: { id: "org-1", name: "Acme", urlKey: "acme" },
  teams: SESSION.teams,
};

const ISSUE = makeIssue({ id: "i1", identifier: "ENG-1", title: "Login schlägt fehl" });

function install() {
  const gql = mockGraphql({
    SidebarUnread: () => ({ notificationsUnreadCount: 0 }),
    IssueTeams: () => ({
      teams: [
        {
          id: "team-eng",
          key: "ENG",
          name: "Engineering",
          parentId: null,
          issueEstimationType: "fibonacci",
        },
      ],
    }),
    TeamMeta: () => ({
      team: TEAM_META.team,
      workflowStates: STATES,
      issueLabels: [],
      cycles: [],
      projects: [],
    }),
    TeamIssues: () => ({ issues: { nodes: [ISSUE], hasNextPage: false, endCursor: null } }),
    IssueDetail: () => ({ issue: ISSUE }),
    IssueComments: () => ({ comments: { nodes: [] } }),
  });
  // Auth-Endpunkt zusätzlich bedienen: GraphQL-Mock fällt für andere URLs durch.
  const base = globalThis.fetch;
  vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) =>
    String(input) === "/auth/me"
      ? Promise.resolve(new Response(JSON.stringify(ME), { status: 200 }))
      : base(input, init),
  );
  return gql;
}

async function renderApp(path: string) {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(
    <Providers client={queryClient}>
      <RouterProvider router={router} />
    </Providers>,
  );
  return router;
}

beforeEach(() => {
  queryClient.clear();
  localStorage.clear();
  setUiState({ createIssueOpen: false, detailsOpen: false });
});
afterEach(() => {
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearToasts();
  resetKnownUsers();
});

describe("Issue-Routen in der Shell", () => {
  it("/acme/team/ENG/all zeigt die Issue-Liste in der Shell", async () => {
    install();
    await renderApp("/acme/team/ENG/all");
    expect(await screen.findByText("Login schlägt fehl")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Issue views" })).toBeInTheDocument();
  });

  it("Taste C öffnet das Create-Modal, auch außerhalb der Issue-Routen", async () => {
    install();
    await renderApp("/acme/inbox");
    await screen.findByRole("navigation", { name: "Main" });
    await userEvent.keyboard("c");
    expect(await screen.findByLabelText("Issue title")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Create issue/ })).toBeDisabled();
  });

  it("/acme/issue/ENG-1 ergänzt den Slug in der URL und zeigt das Detail", async () => {
    install();
    const router = await renderApp("/acme/issue/ENG-1");
    expect(await screen.findByLabelText("Title")).toHaveValue("Login schlägt fehl");
    await waitFor(() =>
      expect(router.state.location.pathname).toBe("/acme/issue/ENG-1/login-schlagt-fehl"),
    );
  });

  it("/acme/my-issues leitet auf Assigned um", async () => {
    install();
    const router = await renderApp("/acme/my-issues");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/my-issues/assigned"));
    expect(await screen.findByRole("navigation", { name: "My issues tabs" })).toBeInTheDocument();
  });

  it("unbekannte My-Issues-Tabs leiten auf Assigned um", async () => {
    install();
    const router = await renderApp("/acme/my-issues/activity");
    await waitFor(() => expect(router.state.location.pathname).toBe("/acme/my-issues/assigned"));
  });
});
