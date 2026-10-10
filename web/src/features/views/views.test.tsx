import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import type { Favorite } from "@/gql/graphql";

import { applyVisibility, groupIssues, mergeDisplay, DEFAULT_DISPLAY } from "./display-options";
import { resolveFavorite } from "./favorites-section";
import { IssueView } from "./issue-view";

afterEach(() => vi.unstubAllGlobals());

const teamRows = [
  {
    id: "team-eng",
    key: "ENG",
    name: "Engineering",
    parentId: null,
    issueEstimationType: "fibonacci",
  },
];

describe("IssueView filter bar", () => {
  it("sends the generated filter JSON to viewIssues", async () => {
    const gql = mockGraphql({
      ViewIssuesList: () => ({
        viewIssues: {
          nodes: [makeIssue({ id: "i1", identifier: "ENG-1" })],
          hasNextPage: false,
          endCursor: null,
        },
      }),
      IssueTeams: () => ({ teams: teamRows }),
      TeamMeta: () => ({
        team: TEAM_META.team,
        workflowStates: TEAM_META.states,
        issueLabels: TEAM_META.labels,
        cycles: [],
        projects: TEAM_META.projects,
      }),
      KnownUsers: () => ({
        users: [{ id: "u-bob", name: "Bob", displayName: "Bob", avatarUrl: null }],
      }),
    });
    await renderWithApp(<IssueView ws="acme" title="Issues" />, newClient(), "/acme/views/issues");
    const user = userEvent.setup();
    await screen.findByText("Issue ENG-1");

    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Assignee" }));
    await user.click(await screen.findByLabelText("Current user"));
    await user.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() =>
      expect(gql.of("ViewIssuesList").at(-1)!.variables.filter).toEqual({
        assignee: { isMe: { eq: true } },
      }),
    );
    expect(screen.getByText(/Assignee is Current user/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Remove filter Assignee" }));
    await waitFor(() =>
      expect(screen.queryByText(/Assignee is Current user/)).not.toBeInTheDocument(),
    );
  });
});

describe("display options", () => {
  it("merges saved values over defaults and ignores invalid ones", () => {
    const o = mergeDisplay({ grouping: "assignee", ordering: "bogus", showSubIssues: false });
    expect(o.grouping).toBe("assignee");
    expect(o.ordering).toBe(DEFAULT_DISPLAY.ordering);
    expect(o.showSubIssues).toBe(false);
  });

  it("hides sub-issues and old completed issues and counts them", () => {
    const now = Date.parse("2026-10-10T00:00:00Z");
    const issues = [
      makeIssue({ id: "a", identifier: "ENG-1" }),
      makeIssue({ id: "b", identifier: "ENG-2", parentId: "a" }),
      makeIssue({
        id: "c",
        identifier: "ENG-3",
        stateId: "s-done",
        completedAt: "2026-01-01T00:00:00Z",
      }),
    ];
    const { visible, hidden } = applyVisibility(
      issues,
      { ...DEFAULT_DISPLAY, showSubIssues: false, showCompletedIssues: "pastWeek" },
      now,
    );
    expect(visible.map((i) => i.id)).toEqual(["a"]);
    expect(hidden).toBe(2);
  });

  it("groups by priority in importance order", () => {
    const issues = [
      makeIssue({ id: "a", identifier: "ENG-1", priority: 0 }),
      makeIssue({ id: "b", identifier: "ENG-2", priority: 1 }),
    ];
    const g = groupIssues(issues, "priority", {
      assignee: () => null,
      project: () => null,
      team: () => null,
      cycle: () => null,
    });
    expect(g.map((x) => x.label)).toEqual(["Urgent", "No priority"]);
  });
});

describe("resolveFavorite", () => {
  const ctx = {
    views: new Map([["v1", { name: "My bugs", slugId: "abc" }]]),
    teams: new Map([["t1", "ENG"]]),
  };
  const fav = (over: Partial<Favorite>): Favorite => ({
    id: "f",
    type: "customView",
    parentId: null,
    folderName: null,
    sortOrder: 1,
    targetId: null,
    predefinedViewType: null,
    predefinedViewTeamId: null,
    ...over,
  });
  it("maps custom and predefined views", () => {
    expect(resolveFavorite(fav({ targetId: "v1" }), "acme", ctx)).toEqual({
      label: "My bugs",
      to: "/acme/view/abc",
    });
    expect(
      resolveFavorite(
        fav({
          type: "predefinedView",
          predefinedViewType: "activeIssues",
          predefinedViewTeamId: "t1",
        }),
        "acme",
        ctx,
      ),
    ).toEqual({ label: "Active · ENG", to: "/acme/team/ENG/active" });
    expect(resolveFavorite(fav({ targetId: "gone" }), "acme", ctx)).toBeNull();
  });
});
