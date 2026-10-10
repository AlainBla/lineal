import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { MyIssuesPage } from "./my-issues-page";
import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { resetKnownUsers } from "./users";

const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", stateId: "s-todo", sortOrder: 1 }),
  makeIssue({ id: "b", identifier: "ENG-2", title: "Zweites", stateId: "s-progress" }),
  makeIssue({ id: "d", identifier: "ENG-4", title: "Im Backlog", stateId: "s-backlog" }),
  makeIssue({ id: "e", identifier: "ENG-5", title: "Fertig", stateId: "s-done" }),
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

describe("Team-Issues", () => {
  it("Active zeigt als Board je Status eine Spalte (unstarted und started)", async () => {
    mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
    });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="active" />);
    const todo = await screen.findByRole("region", { name: "Todo" });
    expect(within(todo).getByText("Erstes")).toBeInTheDocument();
    const progress = screen.getByRole("region", { name: "In Progress" });
    expect(within(progress).getByText("Zweites")).toBeInTheDocument();
    expect(screen.queryByText("Im Backlog")).toBeNull();
    expect(screen.queryByText("Fertig")).toBeNull();
  });

  it("Backlog und All zeigen eine nach Status gruppierte Liste; Umschalter wechselt auf Board", async () => {
    mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
    });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />);
    const list = await screen.findByRole("list", { name: "Issues" });
    for (const t of ["Erstes", "Zweites", "Im Backlog", "Fertig"]) {
      expect(within(list).getByText(t)).toBeInTheDocument();
    }
    expect(within(list).getByRole("region", { name: "Done" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Board layout" }));
    expect(await screen.findByRole("region", { name: "Canceled" })).toBeInTheDocument();
    expect(localStorage.getItem("lineal-issues-layout:ENG:all")).toBe("board");
  });

  it("J fokussiert die erste Zeile, S öffnet den Status-Picker für das fokussierte Issue", async () => {
    const gql = mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
      IssueUpdate: (v) => ({
        issueUpdate: { success: true, issue: { ...ISSUES[2], ...(v.input as object) } },
      }),
    });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="backlog" />);
    await screen.findByText("Im Backlog");
    await userEvent.keyboard("j");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-issue-id", "d"));
    await userEvent.keyboard("s");
    const input = await screen.findByPlaceholderText("Change status…");
    await userEvent.type(input, "2"); // Todo
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({ id: "d", input: { stateId: "s-todo" } });
  });

  it("zeigt Leerzustand, Fehlerzustand und Ladeskelett", async () => {
    mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: [], hasNextPage: false, endCursor: null } }),
    });
    const first = await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="backlog" />);
    expect(screen.getByRole("status", { name: "Loading issues" })).toBeInTheDocument();
    expect(await screen.findByText("No backlog issues")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create issue" })).toBeInTheDocument();
    first.unmount();

    mockGraphql({ TeamMeta: meta, TeamIssues: () => new Error("boom") });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="backlog" />, newClient());
    expect(await screen.findByRole("alert")).toHaveTextContent("boom");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });
});

describe("My Issues", () => {
  it("Assigned lädt mit assigneeId me und gruppiert nach Status; Created filtert nach Ersteller", async () => {
    const mine = makeIssue({
      id: "m1",
      identifier: "ENG-7",
      title: "Meins",
      assigneeId: "user-bob",
    });
    const other = makeIssue({
      id: "m2",
      identifier: "ENG-8",
      title: "Von Alice",
      creatorId: "user-alice",
    });
    const gql = mockGraphql({
      TeamMeta: meta,
      TeamIssues: (v) => ({
        issues: {
          nodes: v.assigneeId === "me" ? [mine] : [mine, other],
          hasNextPage: false,
          endCursor: null,
        },
      }),
    });
    const client = newClient();
    const first = await renderWithApp(<MyIssuesPage ws="acme" tab="assigned" />, client);
    expect(await screen.findByText("Meins")).toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "Unstarted" })).getByText("Meins"),
    ).toBeInTheDocument();
    expect(gql.of("TeamIssues")[0]!.variables).toMatchObject({ assigneeId: "me" });
    first.unmount();

    await renderWithApp(<MyIssuesPage ws="acme" tab="created" />, client);
    expect(await screen.findByText("Meins")).toBeInTheDocument();
    expect(screen.queryByText("Von Alice")).toBeNull();
  });

  it("Subscribed erklärt, dass die API keine Abo-Liste liefert", async () => {
    mockGraphql({});
    await renderWithApp(<MyIssuesPage ws="acme" tab="subscribed" />);
    expect(await screen.findByText("Subscribed issues are not available yet")).toBeInTheDocument();
  });
});
