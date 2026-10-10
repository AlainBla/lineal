import { describe, expect, it } from "vitest";

import {
  applyDrop,
  buildBoardColumns,
  COLUMN_PREFIX,
  estimateOptions,
  filterAssigned,
  filterByView,
  groupByStateType,
  nextFocus,
  previewMove,
  relationInput,
  slugify,
  stateTypeInView,
  subIssueProgress,
  toggleLabel,
} from "./model";
import { makeIssue, STATES, TEAM_META } from "./test-fixtures";

const A = makeIssue({ id: "a", identifier: "ENG-1", stateId: "s-todo", sortOrder: 1 });
const B = makeIssue({ id: "b", identifier: "ENG-2", stateId: "s-todo", sortOrder: 2 });
const C = makeIssue({ id: "c", identifier: "ENG-3", stateId: "s-progress", sortOrder: 5 });
const D = makeIssue({ id: "d", identifier: "ENG-4", stateId: "s-backlog", sortOrder: 1 });
const E = makeIssue({ id: "e", identifier: "ENG-5", stateId: "s-done", sortOrder: 1 });
const ALL = [A, B, C, D, E];

describe("Teamansichten nach Statuskategorie", () => {
  it("Active = unstarted + started, Backlog = backlog, All = alles außer Triage", () => {
    expect(stateTypeInView("started", "active")).toBe(true);
    expect(stateTypeInView("unstarted", "active")).toBe(true);
    expect(stateTypeInView("backlog", "active")).toBe(false);
    expect(stateTypeInView("backlog", "backlog")).toBe(true);
    expect(stateTypeInView("completed", "all")).toBe(true);
    expect(stateTypeInView("triage", "all")).toBe(false);
  });

  it("filtert Issues je Ansicht und ignoriert den Papierkorb", () => {
    expect(filterByView(ALL, "active").map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(filterByView(ALL, "backlog").map((i) => i.id)).toEqual(["d"]);
    expect(
      filterByView([...ALL, makeIssue({ id: "t", identifier: "ENG-9", trashed: true })], "all"),
    ).toHaveLength(5);
  });
});

describe("Board-Spalten", () => {
  it("bildet eine Spalte je Status der Ansicht, Issues nach sortOrder, leere bleiben", () => {
    const cols = buildBoardColumns(STATES, [B, A, C], "active");
    expect(cols.map((c) => c.state.name)).toEqual(["Todo", "In Progress"]);
    expect(cols[0]!.issues.map((i) => i.id)).toEqual(["a", "b"]);
    const all = buildBoardColumns(STATES, [A], "all");
    expect(all.map((c) => c.state.name)).toEqual([
      "Backlog",
      "Todo",
      "In Progress",
      "Done",
      "Canceled",
    ]);
    expect(all[4]!.issues).toEqual([]);
  });
});

describe("Drag & Drop (applyDrop)", () => {
  const cols = () => buildBoardColumns(STATES, ALL, "active");

  it("Drop auf andere Spalte ändert den Status und hängt ans Ende", () => {
    const r = applyDrop(cols(), "a", `${COLUMN_PREFIX}s-progress`);
    expect(r.patch).toEqual({ stateId: "s-progress", sortOrder: 6 });
  });

  it("Drop auf Karte einer anderen Spalte fügt davor ein (zwischen Nachbarn)", () => {
    const withTwo = buildBoardColumns(
      STATES,
      [A, makeIssue({ id: "c2", identifier: "ENG-6", stateId: "s-progress", sortOrder: 7 }), C],
      "active",
    );
    const r = applyDrop(withTwo, "a", "c2");
    expect(r.patch).toEqual({ stateId: "s-progress", sortOrder: 6 });
  });

  it("Drop in leere Spalte ergibt sortOrder 0", () => {
    const only = buildBoardColumns(STATES, [A], "active");
    expect(applyDrop(only, "a", `${COLUMN_PREFIX}s-progress`).patch).toEqual({
      stateId: "s-progress",
      sortOrder: 0,
    });
  });

  it("Umsortieren in der Spalte ändert nur sortOrder (nach unten: hinter das Ziel)", () => {
    const r = applyDrop(cols(), "a", "b");
    expect(r.patch).toEqual({ sortOrder: 3 });
    const up = applyDrop(cols(), "b", "a");
    expect(up.patch).toEqual({ sortOrder: 0 });
  });

  it("keine Änderung bei Drop auf sich selbst, ohne Ziel oder an gleicher Stelle", () => {
    expect(applyDrop(cols(), "a", "a").patch).toBeNull();
    expect(applyDrop(cols(), "a", null).patch).toBeNull();
    expect(applyDrop(cols(), "a", `${COLUMN_PREFIX}s-todo`).patch).toEqual({ sortOrder: 3 });
    expect(applyDrop(cols(), "b", `${COLUMN_PREFIX}s-todo`).patch).toBeNull();
    expect(applyDrop(cols(), "zzz", "a").patch).toBeNull();
  });
});

describe("Tastaturfokus", () => {
  const grid = [["a", "b"], [], ["c", "d", "e"]];
  it("bewegt innerhalb der Spalte und springt über leere Spalten", () => {
    expect(nextFocus(grid, null, "down")).toBe("a");
    expect(nextFocus(grid, "a", "down")).toBe("b");
    expect(nextFocus(grid, "b", "down")).toBe("b");
    expect(nextFocus(grid, "b", "right")).toBe("d");
    expect(nextFocus(grid, "e", "left")).toBe("b");
    expect(nextFocus(grid, "a", "left")).toBe("a");
    expect(nextFocus([[]], null, "down")).toBeNull();
  });
});

describe("Labels, Relationen, Verschieben, Hilfen", () => {
  it("ein Label einer singleSelect-Gruppe ersetzt Geschwister", () => {
    const labels = [
      { ...TEAM_META.labels[0]!, id: "g", name: "Prio", isGroup: true, groupType: "singleSelect" },
      { ...TEAM_META.labels[0]!, id: "x", name: "X", parentId: "g" },
      { ...TEAM_META.labels[0]!, id: "y", name: "Y", parentId: "g" },
      { ...TEAM_META.labels[0]!, id: "z", name: "Z" },
    ];
    expect(toggleLabel(["x", "z"], "y", labels)).toEqual(["z", "y"]);
    expect(toggleLabel(["x"], "z", labels)).toEqual(["x", "z"]);
    expect(toggleLabel(["x", "z"], "x", labels)).toEqual(["z"]);
  });

  it("relationInput dreht „blocked by“ um", () => {
    expect(relationInput("blocking", "me", "o")).toEqual({
      issueId: "me",
      relatedIssueId: "o",
      type: "blocks",
    });
    expect(relationInput("blocked-by", "me", "o")).toEqual({
      issueId: "o",
      relatedIssueId: "me",
      type: "blocks",
    });
    expect(relationInput("duplicate", "me", "o").type).toBe("duplicate");
  });

  it("previewMove nennt nur echte Verluste", () => {
    const issue = makeIssue({
      id: "i",
      identifier: "ENG-1",
      labelIds: ["l-bug", "l-other"],
      projectId: "p-x",
      cycleId: "c-1",
      estimate: 3,
    });
    const { losses } = previewMove(issue, {
      labels: TEAM_META.labels,
      projects: [],
      estimationType: "tShirt",
      sourceEstimationType: "fibonacci",
    });
    expect(losses).toEqual([
      "1 label will be removed",
      "Project will be removed",
      "Cycle will be removed",
      "Estimate will be removed",
    ]);
    expect(
      previewMove(makeIssue({ id: "j", identifier: "ENG-2" }), {
        labels: [],
        projects: [],
        estimationType: "fibonacci",
        sourceEstimationType: "fibonacci",
      }).losses,
    ).toEqual([]);
  });

  it("Fortschritt zählt direkte Kinder ohne canceled/duplicate", () => {
    const kids = [
      makeIssue({ id: "1", identifier: "E-1", stateId: "s-done" }),
      makeIssue({ id: "2", identifier: "E-2", stateId: "s-todo" }),
      makeIssue({ id: "3", identifier: "E-3", stateId: "s-canceled" }),
    ];
    expect(subIssueProgress(kids)).toEqual({ done: 1, total: 2 });
  });

  it("Schätzskalen und Slug", () => {
    expect(estimateOptions("tShirt").map((o) => o.label)).toEqual(["XS", "S", "M", "L", "XL"]);
    expect(estimateOptions("notUsed")).toEqual([]);
    expect(slugify("Login schlägt bei SSO fehl!")).toBe("login-schlagt-bei-sso-fehl");
    expect(slugify("???")).toBe("issue");
  });

  it("My Issues: Gruppen nach Kategorie, Assigned blendet alte erledigte aus", () => {
    const now = Date.parse("2026-10-10T12:00:00Z");
    const recent = makeIssue({
      id: "r",
      identifier: "E-7",
      stateId: "s-done",
      completedAt: "2026-10-10T08:00:00Z",
    });
    const old = makeIssue({
      id: "o",
      identifier: "E-8",
      stateId: "s-done",
      completedAt: "2026-09-01T08:00:00Z",
    });
    expect(filterAssigned([A, recent, old], now).map((i) => i.id)).toEqual(["a", "r"]);
    const groups = groupByStateType([A, C, recent]);
    expect(groups.map((g) => g.label)).toEqual(["Started", "Unstarted", "Completed"]);
  });
});
