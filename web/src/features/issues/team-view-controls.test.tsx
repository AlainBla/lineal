import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { IssueView } from "@/features/views/issue-view";

import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { resetKnownUsers } from "./users";

const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", stateId: "s-todo", priority: 1 }),
  makeIssue({
    id: "b",
    identifier: "ENG-2",
    title: "Zweites",
    stateId: "s-progress",
    parentId: "a",
    priority: 3,
  }),
  makeIssue({ id: "d", identifier: "ENG-4", title: "Im Backlog", stateId: "s-backlog" }),
];

const meta = () => ({
  team: TEAM_META.team,
  workflowStates: TEAM_META.states,
  issueLabels: [],
  cycles: [],
  projects: [],
});

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
  resetKnownUsers();
});

const base = {
  TeamMeta: meta,
  TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
  ViewIssuesList: () => ({
    viewIssues: { nodes: [ISSUES[0]], hasNextPage: false, endCursor: null },
  }),
  ViewPrefsUpdate: () => ({ viewPreferencesUpdate: { hasPersonal: true } }),
};

describe("Team-Seiten: Filterleiste und Display options (F.18)", () => {
  it("Filter-Chip schickt den Filter im Team-Scope an viewIssues und blendet andere Issues aus", async () => {
    const gql = mockGraphql(base);
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />, newClient());
    const user = userEvent.setup();
    await screen.findByText("Im Backlog");
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Priority" }));
    await user.click(await screen.findByLabelText("Urgent"));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(gql.of("ViewIssuesList").length).toBeGreaterThan(0));
    const v = gql.of("ViewIssuesList").at(-1)!.variables;
    expect(v.teamId).toBe(TEAM_META.team.id);
    expect(JSON.stringify(v.filter)).toContain("priority");
    await waitFor(() => expect(screen.queryByText("Im Backlog")).toBeNull());
    expect(screen.getByText("Erstes")).toBeInTheDocument();
  });

  it("⇧V öffnet die Display options; Gruppierung nach Priorität und Sub-Issues ausblenden", async () => {
    const gql = mockGraphql(base);
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />, newClient());
    const user = userEvent.setup();
    await screen.findByText("Im Backlog");
    await user.keyboard("{Shift>}V{/Shift}");
    const dialog = await screen.findByRole("dialog", { name: "Display options" });
    await user.selectOptions(within(dialog).getByLabelText("Grouping"), "priority");
    await user.keyboard("{Escape}");
    const list = await screen.findByRole("list", { name: "Issues" });
    expect(await within(list).findByRole("region", { name: "Urgent" })).toBeInTheDocument();
    expect(within(list).getByRole("region", { name: "No priority" })).toBeInTheDocument();
    await waitFor(() => expect(gql.of("ViewPrefsUpdate")).toHaveLength(1));
    expect(gql.of("ViewPrefsUpdate")[0]!.variables).toMatchObject({
      viewType: "allIssues",
      teamId: TEAM_META.team.id,
      preferences: { grouping: "priority" },
    });

    await user.keyboard("{Shift>}V{/Shift}");
    const again = await screen.findByRole("dialog", { name: "Display options" });
    await user.click(within(again).getByLabelText("Show sub-issues"));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByText("Zweites")).toBeNull());
    expect(screen.getByText(/1 issues hidden by display options/)).toBeInTheDocument();
  });

  it("Standarddarstellung bleibt: Gruppen nach Status, ohne Filter aus dem Team-Cache", async () => {
    const gql = mockGraphql(base);
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />, newClient());
    const list = await screen.findByRole("list", { name: "Issues" });
    expect(await within(list).findByRole("region", { name: "Todo" })).toBeInTheDocument();
    expect(gql.of("ViewIssuesList")).toHaveLength(0);
  });
});

describe("Projekt-Issues mit Filterleiste (F.18)", () => {
  it("IssueView mit Scope verknüpft den Projektfilter per UND mit den Chips", async () => {
    const gql = mockGraphql({
      ...base,
      IssueTeams: () => ({ teams: [] }),
    });
    const scope = { project: { id: { in: ["p1"] } } };
    await renderWithApp(<IssueView ws="acme" title="Issues" scope={scope} />, newClient());
    await screen.findByText("Erstes");
    const first = gql.of("ViewIssuesList")[0]!.variables;
    expect(first.filter).toEqual(scope);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(await screen.findByRole("menuitem", { name: "Priority" }));
    await user.click(await screen.findByLabelText("Urgent"));
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => {
      const f = gql.of("ViewIssuesList").at(-1)!.variables.filter as { and?: unknown[] };
      expect(f.and).toHaveLength(2);
    });
  });
});
