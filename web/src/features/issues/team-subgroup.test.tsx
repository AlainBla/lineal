import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, renderWithApp, TEAM_META } from "./test-fixtures";
import { resetKnownUsers } from "./users";

// LIN-74: Sub-Grouping (zweite Ebene) muss auch auf den Team-Listen wirken.
const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", stateId: "s-todo", assigneeId: "u-bob" }),
  makeIssue({ id: "b", identifier: "ENG-2", title: "Zweites", stateId: "s-todo", assigneeId: null }),
];

function harness() {
  return mockGraphql({
    TeamMeta: () => ({
      team: TEAM_META.team,
      workflowStates: TEAM_META.states,
      issueLabels: TEAM_META.labels,
      cycles: [],
      projects: [],
    }),
    TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
    IssueTeams: () => ({ teams: [] }),
    KnownUsers: () => ({
      users: [{ id: "u-bob", name: "bob", displayName: "Bob", avatarUrl: null }],
    }),
  });
}

async function setSubGrouping(value: string) {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Display" }));
  await user.hover(await screen.findByRole("button", { name: "Sub-grouping" }));
  await user.click(await screen.findByRole("option", { name: value }));
}

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.unstubAllGlobals();
  resetKnownUsers();
});

describe("Team-Liste Sub-Grouping (LIN-74)", () => {
  it("unterteilt die Status-Gruppen nach Assignee (zweite Ebene)", async () => {
    harness();
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />);
    await screen.findByText("Erstes");

    await setSubGrouping("Assignee");

    // Verschachtelte Abschnitte „Todo / <Assignee>" müssen erscheinen.
    await waitFor(() =>
      expect(document.querySelector('section[aria-label*=" / "]')).toBeTruthy(),
    );
    const labels = Array.from(document.querySelectorAll('section[aria-label*=" / "]')).map((s) =>
      s.getAttribute("aria-label"),
    );
    expect(labels).toContain("Todo / Bob");
    expect(labels).toContain("Todo / No assignee");
  });
});
