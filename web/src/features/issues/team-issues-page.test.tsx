import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { MyIssuesPage } from "./my-issues-page";
import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { Toaster } from "./toaster";
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
    await screen.findByRole("region", { name: "Todo" });
    expect(screen.queryByRole("region", { name: "Canceled" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /Hidden columns/ }));
    expect(await screen.findByRole("region", { name: "Canceled" })).toBeInTheDocument();
    expect(localStorage.getItem("lineal-issues-layout:ENG:all")).toBe("board");
  });

  it("sortiert die Liste standardmäßig nach Priorität; Ordering in den Display options wechselt, wird je Team gespeichert und aktiviert Manual-DnD (F.13)", async () => {
    const same = [
      makeIssue({ id: "p0", identifier: "ENG-1", title: "Ohne", stateId: "s-todo", priority: 0 }),
      makeIssue({ id: "p3", identifier: "ENG-2", title: "Mittel", stateId: "s-todo", priority: 3 }),
      makeIssue({
        id: "p1",
        identifier: "ENG-3",
        title: "Dringend",
        stateId: "s-todo",
        priority: 1,
      }),
    ];
    const gql = mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: same, hasNextPage: false, endCursor: null } }),
      ViewPrefsUpdate: () => ({ viewPreferencesUpdate: { hasPersonal: true } }),
    });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="all" />);
    const list = await screen.findByRole("list", { name: "Issues" });
    const titles = () =>
      within(list)
        .getAllByRole("link")
        .map((a) => a.textContent ?? "");
    await waitFor(() => expect(titles()[0]).toContain("Dringend"));
    expect(titles().map((t) => /Dringend|Mittel|Ohne/.exec(t)?.[0])).toEqual([
      "Dringend",
      "Mittel",
      "Ohne",
    ]);
    expect(list.querySelector("[data-sortable-id]")).toBeNull();

    const setOrdering = async (value: string) => {
      await userEvent.keyboard("{Shift>}V{/Shift}");
      const dialog = await screen.findByRole("dialog", { name: "Display options" });
      await userEvent.selectOptions(within(dialog).getByLabelText("Ordering"), value);
      await userEvent.keyboard("{Escape}");
    };
    expect(screen.queryByRole("button", { name: "Ascending order" })).toBeNull();
    await setOrdering("title");
    expect(titles().map((t) => /Dringend|Mittel|Ohne/.exec(t)?.[0])).toEqual([
      "Dringend",
      "Mittel",
      "Ohne",
    ]);
    await setOrdering("manual");
    await waitFor(() =>
      expect(gql.of("ViewPrefsUpdate").at(-1)!.variables).toMatchObject({
        teamId: TEAM_META.team.id,
        preferences: { ordering: "manual" },
      }),
    );
    expect(localStorage.getItem("lineal-issues-ordering:ENG")).toBeNull();
    expect(
      screen.getByRole("list", { name: "Issues" }).querySelectorAll("[data-sortable-id]"),
    ).toHaveLength(3);
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

  it("Shift+D öffnet in der Liste den Datumsdialog, Shift+E ohne Estimates zeigt einen Hinweis (B4.18)", async () => {
    const gql = mockGraphql({
      TeamMeta: () => ({
        ...meta(),
        team: { ...TEAM_META.team, issueEstimationType: "notUsed" },
      }),
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
      IssueUpdate: (v) => ({
        issueUpdate: { success: true, issue: { ...ISSUES[2], ...(v.input as object) } },
      }),
    });
    await renderWithApp(
      <>
        <TeamIssuesPage ws="acme" teamKey="ENG" view="backlog" />
        <Toaster />
      </>,
    );
    await screen.findByText("Im Backlog");
    await userEvent.keyboard("j");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-issue-id", "d"));
    await userEvent.keyboard("{Shift>}E{/Shift}");
    expect(await screen.findByText("Estimates are not enabled for this team.")).toBeInTheDocument();

    await userEvent.keyboard("{Shift>}D{/Shift}");
    const input = await screen.findByLabelText("Due date");
    await userEvent.type(input, "2026-12-24");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toEqual({
      id: "d",
      input: { dueDate: "2026-12-24" },
    });
  });

  it("Board: Strg+Pfeil rechts verschiebt die fokussierte Karte in die nächste Spalte (B4.19)", async () => {
    const gql = mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
      IssueUpdate: (v) => ({
        issueUpdate: { success: true, issue: { ...ISSUES[0], ...(v.input as object) } },
      }),
    });
    await renderWithApp(<TeamIssuesPage ws="acme" teamKey="ENG" view="active" />);
    await screen.findByText("Erstes");
    await userEvent.keyboard("j");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-issue-id", "a"));
    await userEvent.keyboard("{Control>}{ArrowRight}{/Control}");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toMatchObject({
      id: "a",
      input: { stateId: "s-progress" },
    });
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
