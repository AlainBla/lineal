import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { summarizeBatch } from "./bulk-summary";
import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";
import { resetKnownUsers } from "./users";

const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", stateId: "s-todo" }),
  makeIssue({ id: "b", identifier: "ENG-2", title: "Zweites", stateId: "s-progress" }),
  makeIssue({ id: "d", identifier: "ENG-4", title: "Im Backlog", stateId: "s-backlog" }),
  makeIssue({ id: "e", identifier: "ENG-5", title: "Fertig", stateId: "s-done" }),
];

const base = {
  TeamMeta: () => ({
    team: TEAM_META.team,
    workflowStates: TEAM_META.states,
    issueLabels: TEAM_META.labels,
    cycles: [],
    projects: [],
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

describe("summarizeBatch", () => {
  const ident = (id: string) => id.toUpperCase();
  it("meldet Erfolg, Teilerfolg und Totalausfall", () => {
    const issue = ISSUES[0]!;
    expect(summarizeBatch(2, { issues: [issue, issue], failures: [] }, ident)).toEqual({
      message: "Updated 2 issues",
    });
    expect(
      summarizeBatch(
        3,
        {
          issues: [issue],
          failures: [
            { id: "b", code: "forbidden", message: "No access" },
            { id: "c", code: "x", message: "y" },
          ],
        },
        ident,
      ),
    ).toEqual({ message: "Updated 1 of 3. B: No access (+1 more)", tone: "error" });
    expect(
      summarizeBatch(1, { issues: [], failures: [{ id: "a", code: "x", message: "Nope" }] }, ident)
        .message,
    ).toBe("Could not update 1 issue. A: Nope");
  });
});

describe("Mehrfachauswahl", () => {
  it("Cmd-Klick und Shift-Klick wählen aus, die Leiste setzt die Priorität für alle", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: (v) => ({
        issueBatchUpdate: {
          issues: (v.input as { ids: string[] }).ids.map((id) => ({
            ...ISSUES.find((i) => i.id === id)!,
            priority: 1,
          })),
          failures: [],
        },
      }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes");
    const user = userEvent.setup();
    expect(screen.queryByRole("toolbar", { name: "Bulk actions" })).toBeNull();

    await user.keyboard("{Control>}");
    await user.click(screen.getByText("Im Backlog"));
    await user.keyboard("{/Control}");
    await user.keyboard("{Shift>}");
    await user.click(screen.getByText("Zweites"));
    await user.keyboard("{/Shift}");

    const bar = await screen.findByRole("toolbar", { name: "Bulk actions" });
    expect(within(bar).getByText("3 selected")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select ENG-1" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Select ENG-5" })).not.toBeChecked();

    await user.click(within(bar).getByRole("button", { name: "Bulk Priority" }));
    await user.click(await screen.findByRole("option", { name: /Urgent/ }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    const input = (gql.of("IssueBatchUpdate")[0]!.variables as { input: Record<string, unknown> })
      .input;
    expect([...(input.ids as string[])].sort()).toEqual(["a", "b", "d"]);
    expect(input.priority).toBe(1);
    expect(await screen.findByText("Updated 3 issues")).toBeInTheDocument();
    // Nach Erfolg ist die Auswahl leer.
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Bulk actions" })).toBeNull());
  });

  it("X markiert das fokussierte Issue, Escape hebt die Auswahl auf", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    await screen.findByText("Im Backlog");
    const user = userEvent.setup();
    await user.keyboard("j");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-issue-id", "d"));
    await user.keyboard("x");
    const bar = await screen.findByRole("toolbar", { name: "Bulk actions" });
    expect(within(bar).getByText("1 selected")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Bulk actions" })).toBeNull());
  });

  it("Teilerfolg: Fehler-Toast mit Kennung, Auswahl bleibt bestehen", async () => {
    mockGraphql({
      ...base,
      IssueBatchUpdate: () => ({
        issueBatchUpdate: {
          issues: [{ ...ISSUES[0]!, assigneeId: null }],
          failures: [{ id: "b", code: "forbidden", message: "You cannot edit this issue." }],
        },
      }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes");
    const user = userEvent.setup();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-2" }));
    await user.click(screen.getByRole("button", { name: "Bulk Assignee" }));
    await user.click(await screen.findByRole("option", { name: /No assignee/ }));
    expect(
      await screen.findByText("Updated 1 of 2. ENG-2: You cannot edit this issue."),
    ).toBeInTheDocument();
    expect(screen.getByRole("toolbar", { name: "Bulk actions" })).toBeInTheDocument();
  });

  it("Löschen verlangt eine Bestätigung und sendet delete", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: () => ({ issueBatchUpdate: { issues: [ISSUES[0]!], failures: [] } }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes");
    const user = userEvent.setup();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
    await user.click(screen.getByRole("button", { name: "Bulk delete" }));
    expect(gql.of("IssueBatchUpdate")).toHaveLength(0);
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    expect(
      (gql.of("IssueBatchUpdate")[0]!.variables as { input: Record<string, unknown> }).input,
    ).toEqual({ ids: ["a"], delete: true });
  });
});
