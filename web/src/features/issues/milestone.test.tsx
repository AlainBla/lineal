import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";
import { resetKnownUsers } from "./users";

// F.07: Meilenstein in der Bulk-Leiste und in Zeilen.
const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", projectId: "p-1" }),
  makeIssue({
    id: "b",
    identifier: "ENG-2",
    title: "Zweites",
    projectId: "p-1",
    projectMilestoneId: "m-alpha",
  }),
  makeIssue({ id: "c", identifier: "ENG-3", title: "Ohne Projekt" }),
];

const base = {
  TeamMeta: () => ({
    team: TEAM_META.team,
    workflowStates: TEAM_META.states,
    issueLabels: TEAM_META.labels,
    cycles: [],
    projects: TEAM_META.projects,
  }),
  TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
  IssueTeams: () => ({ teams: [] }),
};

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearToasts();
  resetKnownUsers();
});

const page = () => (
  <>
    <TeamIssuesPage ws="acme" teamKey="ENG" view="all" />
    <Toaster />
  </>
);

describe("Meilenstein am Issue (F.07)", () => {
  it("zeigt den Meilenstein in der Zeile", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    const row = (await screen.findByText("Zweites")).closest("[role=listitem], li, div")!;
    await waitFor(() => expect(row.parentElement?.textContent).toContain("Alpha"));
  });

  it("Bulk: Meilenstein für Issues desselben Projekts, sonst deaktiviert", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: (v) => ({
        issueBatchUpdate: {
          issues: (v.input as { ids: string[] }).ids.map((id) => ({
            ...ISSUES.find((i) => i.id === id)!,
            projectMilestoneId: "m-beta",
          })),
          failures: [],
        },
      }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes");
    const user = userEvent.setup();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-3" }));
    const bar = await screen.findByRole("toolbar", { name: "Bulk actions" });
    // Ein Issue ohne Projekt in der Auswahl: keine Meilenstein-Aktion.
    expect(within(bar).getByRole("button", { name: "Bulk Milestone" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-3" }));
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-2" }));
    await waitFor(() =>
      expect(within(bar).getByRole("button", { name: "Bulk Milestone" })).toBeEnabled(),
    );
    await user.click(within(bar).getByRole("button", { name: "Bulk Milestone" }));
    const picker = await screen.findByRole("dialog");
    await user.click(within(picker).getByRole("option", { name: /Beta/ }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    expect(gql.of("IssueBatchUpdate")[0]!.variables).toEqual({
      input: { ids: ["a", "b"], projectMilestoneId: "m-beta" },
    });
  });
});
