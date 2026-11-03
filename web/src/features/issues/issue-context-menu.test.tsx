import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { closeIssueMenu } from "./context-menu-store";
import { IssueContextMenuHost } from "./issue-context-menu";
import { duePresets } from "./model";
import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts } from "./toast-store";
import { Toaster } from "./toaster";
import { resetKnownUsers } from "./users";

const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes Ticket", stateId: "s-todo" }),
  makeIssue({ id: "b", identifier: "ENG-2", title: "Zweites", stateId: "s-progress" }),
  makeIssue({ id: "d", identifier: "ENG-4", title: "Im Backlog", stateId: "s-backlog" }),
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
  IssueSubscribed: () => ({ issue: { subscribed: false } }),
};

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  detach = attachShortcuts();
});
afterEach(() => {
  closeIssueMenu();
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearToasts();
  resetKnownUsers();
});

const page = () => (
  <>
    <TeamIssuesPage ws="acme" teamKey="ENG" view="all" />
    <IssueContextMenuHost />
    <Toaster />
  </>
);

const rowOf = (text: string) => screen.getByText(text).closest("a")!;

async function openMenu(text: string) {
  rowOf(text).focus();
  fireEvent.contextMenu(rowOf(text), { clientX: 120, clientY: 80 });
  return screen.findByRole("menu");
}

describe("Kontextmenü auf Tickets", () => {
  it("Rechtsklick öffnet das Menü mit Eigenschaften, Kopieren, Archivieren und Löschen", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const menu = await openMenu("Erstes Ticket");
    for (const name of [
      /^Status…/,
      /^Priority…/,
      /^Assignee…/,
      /^Labels…/,
      /^Project…/,
      /^Due date/,
      /^Copy/,
      /^Archive/,
      /^Delete…/,
    ])
      expect(within(menu).getByRole("menuitem", { name })).toBeInTheDocument();
    // Kürzel aus der Registry werden angezeigt (Status = S).
    expect(
      within(within(menu).getByRole("menuitem", { name: /^Status…/ })).getByText("S"),
    ).toBeTruthy();
  });

  it("Menü navigiert nicht und lässt die Zeile unverändert", async () => {
    mockGraphql(base);
    const { router } = await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    await openMenu("Erstes Ticket");
    expect(router.state.location.pathname).toBe("/acme/team/ENG/all");
  });

  it("Status ändern ruft IssueUpdate mit der Issue-ID", async () => {
    const gql = mockGraphql({
      ...base,
      IssueUpdate: (v) => ({ issueUpdate: { issue: { ...ISSUES[0]!, id: v.id as string } } }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await openMenu("Erstes Ticket");
    await user.click(screen.getByRole("menuitem", { name: /^Status…/ }));
    await user.click(await screen.findByRole("option", { name: /In Progress/ }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toMatchObject({
      id: "a",
      input: { stateId: "s-progress" },
    });
  });

  it("Priority und Assignee setzen die Felder des angeklickten Issues", async () => {
    const gql = mockGraphql({
      ...base,
      IssueUpdate: (v) => ({ issueUpdate: { issue: { ...ISSUES[1]!, id: v.id as string } } }),
    });
    await renderWithApp(page());
    await screen.findByText("Zweites");
    const user = userEvent.setup();
    await openMenu("Zweites");
    await user.click(screen.getByRole("menuitem", { name: /^Priority…/ }));
    await user.click(await screen.findByRole("option", { name: /Urgent/ }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables).toMatchObject({ id: "b", input: { priority: 1 } });

    await openMenu("Zweites");
    await user.click(screen.getByRole("menuitem", { name: /^Assign to me/ }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(2));
    expect(gql.of("IssueUpdate")[1]!.variables).toMatchObject({
      id: "b",
      input: { assigneeId: "user-bob" },
    });
  });

  it("Due-date-Untermenü setzt eine Voreinstellung", async () => {
    const gql = mockGraphql({
      ...base,
      IssueUpdate: (v) => ({ issueUpdate: { issue: { ...ISSUES[0]!, id: v.id as string } } }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await openMenu("Erstes Ticket");
    await user.click(screen.getByRole("menuitem", { name: /^Due date/ }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "In one week" }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    const week = duePresets(new Date()).find((p) => p.id === "in-1-week")!.date;
    expect(gql.of("IssueUpdate")[0]!.variables).toMatchObject({
      id: "a",
      input: { dueDate: week },
    });
  });

  it("Copy-Aktionen schreiben Kennung, Link, Titel und Branchnamen in die Zwischenablage", async () => {
    mockGraphql(base);
    window.history.pushState({}, "", "/acme/team/ENG/all");
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    const copy = async (name: string) => {
      await openMenu("Erstes Ticket");
      await user.click(screen.getByRole("menuitem", { name: /^Copy/ }));
      fireEvent.click(await screen.findByRole("menuitem", { name: new RegExp(`^${name}`) }));
    };
    await copy("ID");
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("ENG-1"));
    await copy("Link");
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(`${window.location.origin}/acme/issue/ENG-1`),
    );
    await copy("Title");
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("Erstes Ticket"));
    await copy("Git branch name");
    // Der Branchname kommt unverändert vom Server (`Issue.branchName`, identisch zu MCP).
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("alice/eng-1"));
  });

  it("Löschen fragt nach und sendet erst nach Bestätigung", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: () => ({ issueBatchUpdate: { issues: [ISSUES[0]!], failures: [] } }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await openMenu("Erstes Ticket");
    await user.click(screen.getByRole("menuitem", { name: /^Delete…/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Delete 1 issue?")).toBeInTheDocument();
    expect(gql.of("IssueBatchUpdate")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    expect(gql.of("IssueBatchUpdate")[0]!.variables).toMatchObject({
      input: { ids: ["a"], delete: true },
    });
  });

  it("Archivieren sendet archive ohne Rückfrage", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: () => ({ issueBatchUpdate: { issues: [ISSUES[0]!], failures: [] } }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await openMenu("Erstes Ticket");
    await user.click(screen.getByRole("menuitem", { name: /^Archive/ }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    expect(gql.of("IssueBatchUpdate")[0]!.variables).toMatchObject({
      input: { ids: ["a"], archive: true },
    });
  });

  it("Shift+F10 auf der fokussierten Zeile öffnet das Menü", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    rowOf("Zweites").focus();
    fireEvent.keyDown(rowOf("Zweites"), { key: "F10", shiftKey: true });
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByText("ENG-2")).toBeInTheDocument();
  });

  it("Kontextmenü-Taste öffnet das Menü und Escape schließt es", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    rowOf("Erstes Ticket").focus();
    fireEvent.keyDown(rowOf("Erstes Ticket"), { key: "ContextMenu" });
    await screen.findByRole("menu");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("Rechtsklick in die Mehrfachauswahl wirkt auf alle markierten Issues", async () => {
    const gql = mockGraphql({
      ...base,
      IssueBatchUpdate: (v) => ({
        issueBatchUpdate: {
          issues: (v.input as { ids: string[] }).ids.map((id) => ISSUES.find((i) => i.id === id)!),
          failures: [],
        },
      }),
    });
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-2" }));
    const menu = await openMenu("Zweites");
    expect(within(menu).getByText("2 issues")).toBeInTheDocument();
    await user.click(within(menu).getByRole("menuitem", { name: /^Priority…/ }));
    await user.click(await screen.findByRole("option", { name: /Urgent/ }));
    await waitFor(() => expect(gql.of("IssueBatchUpdate")).toHaveLength(1));
    const input = (gql.of("IssueBatchUpdate")[0]!.variables as { input: Record<string, unknown> })
      .input;
    expect([...(input.ids as string[])].sort()).toEqual(["a", "b"]);
    expect(input.priority).toBe(1);
  });

  it("Rechtsklick auf eine nicht markierte Zeile betrifft nur diese", async () => {
    mockGraphql(base);
    await renderWithApp(page());
    await screen.findByText("Erstes Ticket");
    const user = userEvent.setup();
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
    await user.click(screen.getByRole("checkbox", { name: "Select ENG-2" }));
    const menu = await openMenu("Im Backlog");
    expect(within(menu).getByText("ENG-4")).toBeInTheDocument();
    expect(within(menu).queryByText("2 issues")).toBeNull();
  });

  describe("Relations", () => {
    const relHandlers = {
      ...base,
      IssueDetail: (v: Record<string, unknown>) => ({
        issue: ISSUES.find((i) => i.identifier === v.id) ?? ISSUES[2],
      }),
      IssueRelationCreate: () => ({ issueRelationCreate: { success: true } }),
    };

    async function addRelation(item: RegExp, target: string, openOn = "Erstes Ticket") {
      const user = userEvent.setup();
      const menu = await openMenu(openOn);
      await user.click(within(menu).getByRole("menuitem", { name: /^Relations/ }));
      fireEvent.click(await screen.findByRole("menuitem", { name: item }));
      await user.type(await screen.findByLabelText("Issue identifier"), target);
      await user.click(screen.getByRole("button", { name: "Add relation" }));
    }

    it.each([
      [/^Blocks…/, { issueId: "a", relatedIssueId: "d", type: "blocks" }],
      [/^Blocked by…/, { issueId: "d", relatedIssueId: "a", type: "blocks" }],
      [/^Related to…/, { issueId: "a", relatedIssueId: "d", type: "related" }],
      [/^Mark as duplicate of…/, { issueId: "a", relatedIssueId: "d", type: "duplicate" }],
    ])("%s öffnet den Dialog und legt die Relation an", async (item, input) => {
      const gql = mockGraphql(relHandlers);
      await renderWithApp(page());
      await screen.findByText("Erstes Ticket");
      await addRelation(item, "ENG-4");
      await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(1));
      expect(gql.of("IssueRelationCreate")[0]!.variables).toEqual({ input });
      await waitFor(() => expect(screen.queryByLabelText("Issue identifier")).toBeNull());
    });

    it("Mehrfachauswahl: eine Relation je Issue, Selbstbezug übersprungen, kein Duplicate", async () => {
      const gql = mockGraphql(relHandlers);
      await renderWithApp(page());
      await screen.findByText("Erstes Ticket");
      const user = userEvent.setup();
      await user.click(screen.getByRole("checkbox", { name: "Select ENG-1" }));
      await user.click(screen.getByRole("checkbox", { name: "Select ENG-2" }));
      await user.click(screen.getByRole("checkbox", { name: "Select ENG-4" }));
      const menu = await openMenu("Zweites");
      await user.click(within(menu).getByRole("menuitem", { name: /^Relations/ }));
      await screen.findByRole("menuitem", { name: /^Blocks…/ });
      expect(screen.queryByRole("menuitem", { name: /duplicate/ })).toBeNull();
      fireEvent.click(screen.getByRole("menuitem", { name: /^Blocks…/ }));
      await user.type(await screen.findByLabelText("Issue identifier"), "ENG-4");
      await user.click(screen.getByRole("button", { name: "Add relation" }));
      await waitFor(() => expect(gql.of("IssueRelationCreate")).toHaveLength(2));
      const inputs = gql
        .of("IssueRelationCreate")
        .map((c) => (c.variables as { input: Record<string, string> }).input);
      expect(inputs.map((i) => i.issueId).sort()).toEqual(["a", "b"]);
      expect(inputs.every((i) => i.relatedIssueId === "d" && i.type === "blocks")).toBe(true);
    });
  });
});
