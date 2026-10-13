import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  makeIssue,
  mockGraphql,
  newClient,
  renderWithApp,
  TEAM_META,
} from "@/features/issues/test-fixtures";
import type { CustomView, Favorite } from "@/gql/graphql";

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
      to: "/acme/view/my-bugs-abc",
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

describe("IssueView mit gespeicherter View (WP31)", () => {
  const view = (filterData: unknown): CustomView =>
    ({
      id: "v-1",
      slugId: "abc",
      name: "My bugs",
      description: null,
      icon: null,
      color: null,
      modelName: "Issue",
      filterData,
      ownerId: "u-alice",
      ownerName: "Alice",
      shared: true,
      teamId: "team-eng",
      teamKey: "ENG",
      projectId: null,
      initiativeId: null,
      sortOrder: 0,
      version: 3,
      canEdit: true,
      createdAt: "2026-10-01T00:00:00Z",
      updatedAt: "2026-10-01T00:00:00Z",
      archivedAt: null,
    }) as unknown as CustomView;

  const handlers = (extra: Record<string, (v: Record<string, unknown>) => unknown> = {}) => ({
    ViewIssuesList: () => ({
      viewIssues: {
        nodes: [makeIssue({ id: "i1", identifier: "ENG-1" })],
        hasNextPage: false,
        endCursor: null,
      },
    }),
    ViewPrefs: () => ({ viewPreferences: { values: {}, hasPersonal: false, hasDefault: false } }),
    ViewSubscriptionGet: () => ({ viewSubscription: null }),
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
    ...extra,
  });

  it("zeigt filterData als Chips, wendet die View unverändert serverseitig an", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <IssueView
        ws="acme"
        title="My bugs"
        view={view({ and: [{ assignee: { isMe: { eq: true } } }, { priority: { in: [1] } }] })}
      />,
      newClient(),
      "/acme/view/abc",
    );
    await screen.findByText("Issue ENG-1");
    expect(screen.getByText(/Assignee is Current user/)).toBeInTheDocument();
    expect(screen.getByText(/Priority is Urgent/)).toBeInTheDocument();
    const last = gql.of("ViewIssuesList").at(-1)!.variables;
    expect(last.customViewId).toBe("v-1");
    expect(last.filter).toBeUndefined();
  });

  it("Chip entfernen: Chip-Filter ersetzt filterData, Team-Scope bleibt, Speichern möglich", async () => {
    const saved: Array<Record<string, unknown>> = [];
    const gql = mockGraphql(
      handlers({
        ViewUpdate: (v) => {
          saved.push(v);
          return { customViewUpdate: { success: true } };
        },
      }),
    );
    await renderWithApp(
      <IssueView
        ws="acme"
        title="My bugs"
        view={view({ and: [{ assignee: { isMe: { eq: true } } }, { priority: { in: [1] } }] })}
      />,
      newClient(),
      "/acme/view/abc",
    );
    const user = userEvent.setup();
    await screen.findByText("Issue ENG-1");
    await user.click(screen.getByRole("button", { name: "Remove filter Priority" }));
    await waitFor(() => {
      const v = gql.of("ViewIssuesList").at(-1)!.variables;
      expect(v.customViewId).toBeUndefined();
      expect(v.filter).toEqual({
        and: [{ assignee: { isMe: { eq: true } } }, { team: { id: { in: ["team-eng"] } } }],
      });
    });
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({
      id: "v-1",
      input: { filterData: { assignee: { isMe: { eq: true } } }, expectedVersion: 3 },
    });
  });

  it("Save sendet die Basisversion des Entwurfs, nicht die live aktualisierte (B4.04)", async () => {
    const saved: Array<Record<string, unknown>> = [];
    mockGraphql(
      handlers({
        ViewUpdate: (v) => {
          saved.push(v);
          return { customViewUpdate: { success: true } };
        },
      }),
    );
    const base = view({ and: [{ assignee: { isMe: { eq: true } } }, { priority: { in: [1] } }] });
    function Live() {
      const [version, setVersion] = useState(3);
      return (
        <>
          <button onClick={() => setVersion(4)}>live update</button>
          <IssueView ws="acme" title="My bugs" view={{ ...base, version }} />
        </>
      );
    }
    await renderWithApp(<Live />, newClient(), "/acme/view/abc");
    const user = userEvent.setup();
    await screen.findByText("Issue ENG-1");
    await user.click(screen.getByRole("button", { name: "Remove filter Priority" }));
    await user.click(screen.getByRole("button", { name: "live update" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect((saved[0]!.input as Record<string, unknown>).expectedVersion).toBe(3);
  });

  it("Chip bearbeiten: Dialog mit vorbelegtem Wert", async () => {
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <IssueView ws="acme" title="My bugs" view={view({ priority: { in: [1] } })} />,
      newClient(),
      "/acme/view/abc",
    );
    const user = userEvent.setup();
    await screen.findByText("Issue ENG-1");
    await user.click(screen.getByRole("button", { name: "Edit filter Priority" }));
    const urgent = await screen.findByLabelText("Urgent");
    expect(urgent).toBeChecked();
    await user.click(urgent);
    await user.click(screen.getByLabelText("High"));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(gql.of("ViewIssuesList").at(-1)!.variables.filter).toEqual({
        and: [{ priority: { in: [2] } }, { team: { id: { in: ["team-eng"] } } }],
      }),
    );
  });

  it("nicht darstellbare Bedingungen bleiben als erweiterter Filter erhalten", async () => {
    const or = { or: [{ priority: { in: [1] } }, { estimate: { null: true } }] };
    const gql = mockGraphql(handlers());
    await renderWithApp(
      <IssueView
        ws="acme"
        title="My bugs"
        view={view({ and: [{ assignee: { isMe: { eq: true } } }, or] })}
      />,
      newClient(),
      "/acme/view/abc",
    );
    const user = userEvent.setup();
    await screen.findByText("Issue ENG-1");
    expect(screen.getByText(/Advanced filter \(1\)/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove filter Assignee" }));
    await waitFor(() => {
      const f = gql.of("ViewIssuesList").at(-1)!.variables.filter as { and: unknown[] };
      expect(f.and).toContainEqual(or);
      expect(f.and).not.toContainEqual({ assignee: { isMe: { eq: true } } });
    });
  });

  it("Glocke: abonnieren mit Standardtyp", async () => {
    const calls: Array<Record<string, unknown>> = [];
    mockGraphql(
      handlers({
        ViewSubscriptionSet: (v) => {
          calls.push(v);
          return { viewSubscriptionUpdate: { active: true } };
        },
      }),
    );
    await renderWithApp(
      <IssueView ws="acme" title="My bugs" view={view({ priority: { in: [1] } })} />,
      newClient(),
      "/acme/view/abc",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Set up notifications" }));
    await user.click(await screen.findByLabelText("New issue in this view"));
    await waitFor(() =>
      expect(calls).toEqual([{ id: "v-1", active: true, types: ["issueAddedToView"] }]),
    );
  });

  it("Eigentümerwechsel: customViewUpdate mit ownerId", async () => {
    const calls: Array<Record<string, unknown>> = [];
    mockGraphql(
      handlers({
        ViewUpdate: (v) => {
          calls.push(v);
          return { customViewUpdate: { success: true } };
        },
      }),
    );
    await renderWithApp(
      <IssueView ws="acme" title="My bugs" view={view({ priority: { in: [1] } })} />,
      newClient(),
      "/acme/view/abc",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Transfer ownership" }));
    await user.selectOptions(await screen.findByLabelText("New owner"), "u-bob");
    await user.click(screen.getByRole("button", { name: "Transfer" }));
    await waitFor(() =>
      expect(calls).toEqual([{ id: "v-1", input: { ownerId: "u-bob", expectedVersion: 3 } }]),
    );
  });
});

describe("viewPath (B4.20)", () => {
  it("hängt den Namens-Slug vor die slugId", async () => {
    const { viewPath } = await import("./view-path");
    expect(viewPath("acme", { name: "Offene Bugs (Web)", slugId: "77aa00bb11cc" })).toBe(
      "/acme/view/offene-bugs-web-77aa00bb11cc",
    );
  });
});
