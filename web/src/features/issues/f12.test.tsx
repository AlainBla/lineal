import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { attachShortcuts, shortcuts } from "@/lib/shortcuts";

import { useIssueGlobalShortcuts } from "./global-shortcuts";
import { useIssueActions } from "./issue-actions";
import { MyIssuesPage } from "./my-issues-page";
import { TeamIssuesPage } from "./team-issues-page";
import { makeIssue, mockGraphql, newClient, renderWithApp, TEAM_META } from "./test-fixtures";
import { clearToasts, showToast } from "./toast-store";
import { Toaster } from "./toaster";
import { clearUndo, inverseBulk, inverseInput, recordUndo, redoLast, undoLast } from "./undo";
import { resetKnownUsers } from "./users";

const ISSUES = [
  makeIssue({ id: "a", identifier: "ENG-1", title: "Erstes", stateId: "s-todo", priority: 2 }),
  makeIssue({ id: "b", identifier: "ENG-2", title: "Zweites", stateId: "s-progress" }),
];

const meta = () => ({
  team: TEAM_META.team,
  workflowStates: TEAM_META.states,
  issueLabels: TEAM_META.labels,
  cycles: [],
  projects: [],
});

let detach: () => void;
beforeEach(() => {
  localStorage.clear();
  clearUndo();
  detach = attachShortcuts();
});
afterEach(() => {
  detach();
  vi.unstubAllGlobals();
  shortcuts.reset();
  clearToasts();
  clearUndo();
  resetKnownUsers();
});

describe("Undo-Stapel", () => {
  it("undo und redo arbeiten in umgekehrter Reihenfolge; neue Aktion leert Redo", () => {
    const log: string[] = [];
    recordUndo({ label: "a", undo: () => log.push("undo a"), redo: () => log.push("redo a") });
    recordUndo({ label: "b", undo: () => log.push("undo b"), redo: () => log.push("redo b") });
    expect(undoLast()?.label).toBe("b");
    expect(undoLast()?.label).toBe("a");
    expect(undoLast()).toBeNull();
    expect(redoLast()?.label).toBe("a");
    recordUndo({ label: "c", undo: () => log.push("undo c"), redo: () => log.push("redo c") });
    expect(redoLast()).toBeNull();
    expect(log).toEqual(["undo b", "undo a", "redo a"]);
  });

  it("inverseInput liefert die vorherigen Werte, Titel und Beschreibung bleiben außen vor", () => {
    expect(inverseInput(ISSUES[0]!, { priority: 1, title: "x" })).toEqual({ priority: 2 });
    expect(inverseInput(ISSUES[0]!, { title: "x" })).toBeNull();
    expect(inverseBulk(ISSUES, { priority: 1 })).toEqual([
      { id: "a", input: { priority: 2 } },
      { id: "b", input: { priority: ISSUES[1]!.priority } },
    ]);
    expect(inverseBulk(ISSUES, { delete: true })).toBeNull();
    expect(inverseBulk(ISSUES, { teamId: "t" })).toBeNull();
    expect(inverseBulk(ISSUES, { addedLabelIds: ["l1"] })?.[0]?.input).toEqual({ labelIds: [] });
  });
});

function Harness() {
  useIssueGlobalShortcuts();
  const actions = useIssueActions();
  return (
    <>
      <button type="button" onClick={() => actions.setPriority(ISSUES[0]!, 1)}>
        Urgent
      </button>
      <input aria-label="Text" />
      <Toaster />
    </>
  );
}

describe("Ctrl Z / Ctrl ⇧ Z", () => {
  it("macht die letzte Feldänderung rückgängig und stellt sie wieder her", async () => {
    const gql = mockGraphql({
      IssueUpdate: (v) => ({
        issueUpdate: { success: true, issue: { ...ISSUES[0], ...(v.input as object) } },
      }),
    });
    await renderWithApp(<Harness />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Urgent" }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(1));
    expect(gql.of("IssueUpdate")[0]!.variables.input).toEqual({ priority: 1 });

    await user.keyboard("{Control>}z{/Control}");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(2));
    expect(gql.of("IssueUpdate")[1]!.variables).toMatchObject({
      id: "a",
      input: { priority: 2 },
    });
    expect(await screen.findByText(/Undid ENG-1/)).toBeInTheDocument();

    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}");
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(3));
    expect(gql.of("IssueUpdate")[2]!.variables.input).toEqual({ priority: 1 });
  });

  it("in Texteingaben bleibt Ctrl Z beim Browser", async () => {
    const gql = mockGraphql({
      IssueUpdate: () => ({ issueUpdate: { success: true, issue: ISSUES[0] } }),
    });
    recordUndo({ label: "x", undo: () => void gql.fn("undo"), redo: () => {} });
    await renderWithApp(<Harness />);
    const user = userEvent.setup();
    await user.click(screen.getByLabelText("Text"));
    await user.keyboard("{Control>}z{/Control}");
    expect(gql.fn).not.toHaveBeenCalled();
  });

  it("Ctrl Alt X schließt alle Toasts", async () => {
    mockGraphql({});
    await renderWithApp(<Harness />);
    showToast({ message: "Eins" });
    showToast({ message: "Zwei" });
    expect(await screen.findByText("Zwei")).toBeInTheDocument();
    await userEvent.keyboard("{Control>}{Alt>}x{/Alt}{/Control}");
    await waitFor(() => expect(screen.queryByText("Zwei")).toBeNull());
    expect(screen.queryByText("Eins")).toBeNull();
  });
});

describe("Bulk-Toast mit Undo", () => {
  it("Undo im Toast stellt die vorherigen Werte je Issue wieder her", async () => {
    const gql = mockGraphql({
      TeamMeta: meta,
      TeamIssues: () => ({ issues: { nodes: ISSUES, hasNextPage: false, endCursor: null } }),
      IssueTeams: () => ({ teams: [] }),
      IssueBatchUpdate: (v) => ({
        issueBatchUpdate: {
          issues: (v.input as { ids: string[] }).ids.map((id) => ({
            ...ISSUES.find((i) => i.id === id)!,
            priority: 1,
          })),
          failures: [],
        },
      }),
      IssueUpdate: (v) => ({ issueUpdate: { success: true, issue: ISSUES[0] ?? v } }),
    });
    await renderWithApp(
      <>
        <TeamIssuesPage ws="acme" teamKey="ENG" view="all" />
        <Toaster />
      </>,
    );
    await screen.findByText("Erstes");
    const user = userEvent.setup();
    await user.keyboard("{Control>}a{/Control}");
    const bar = await screen.findByRole("toolbar", { name: "Bulk actions" });
    expect(within(bar).getByText("2 selected")).toBeInTheDocument();
    await user.click(within(bar).getByRole("button", { name: "Bulk Priority" }));
    await user.click(await screen.findByRole("option", { name: /Urgent/ }));
    expect(await screen.findByText("Updated 2 issues")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(gql.of("IssueUpdate")).toHaveLength(2));
    const inputs = gql
      .of("IssueUpdate")
      .map((c) => ({ id: c.variables.id, input: c.variables.input }));
    expect(inputs).toContainEqual({ id: "a", input: { priority: 2 } });
    expect(inputs).toContainEqual({ id: "b", input: { priority: ISSUES[1]!.priority } });
  });
});

describe("My issues: Auswahl und Board", () => {
  const mine = [
    makeIssue({ id: "m1", identifier: "ENG-7", title: "Meins", assigneeId: "user-bob" }),
    makeIssue({
      id: "m2",
      identifier: "ENG-8",
      title: "Läuft",
      assigneeId: "user-bob",
      stateId: "s-progress",
      stateType: "started",
      stateName: "In Progress",
    }),
  ];
  const handlers = () => ({
    TeamMeta: meta,
    TeamIssues: () => ({ issues: { nodes: mine, hasNextPage: false, endCursor: null } }),
  });

  it("X wählt aus, Ctrl A wählt alle, Escape leert", async () => {
    mockGraphql(handlers());
    await renderWithApp(<MyIssuesPage ws="acme" tab="assigned" />, newClient());
    await screen.findByText("Meins");
    const user = userEvent.setup();
    await user.keyboard("j");
    await waitFor(() => expect(document.activeElement).toHaveAttribute("data-issue-id"));
    await user.keyboard("x");
    expect(await screen.findByText("1 selected")).toBeInTheDocument();
    await user.keyboard("{Control>}a{/Control}");
    expect(await screen.findByText("2 selected")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Bulk actions" })).toBeNull());
  });

  it("Ctrl B und der Umschalter wechseln zum Board mit Spalten je Statuskategorie", async () => {
    mockGraphql(handlers());
    await renderWithApp(<MyIssuesPage ws="acme" tab="assigned" />, newClient());
    await screen.findByText("Meins");
    const user = userEvent.setup();
    await user.keyboard("{Control>}b{/Control}");
    const started = await screen.findByRole("region", { name: "Started" });
    expect(within(started).getByText("Läuft")).toBeInTheDocument();
    expect(localStorage.getItem("lineal-issues-layout:my:assigned")).toBe("board");
    await user.click(screen.getByRole("button", { name: "List layout" }));
    expect(await screen.findByRole("list", { name: "Issues" })).toBeInTheDocument();
  });
});
