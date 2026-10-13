import { describe, expect, it } from "vitest";

import { makeIssue } from "@/features/issues/test-fixtures";

import { DEFAULT_DISPLAY, defaultDirection, sortIssues } from "./display-options";

const ids = (xs: { id: string }[]) => xs.map((i) => i.id);

describe("sortIssues (docs/05 §2.5)", () => {
  it("Priority descending: No priority bleibt zuletzt", () => {
    const issues = [
      makeIssue({ id: "none", identifier: "ENG-1", priority: 0 }),
      makeIssue({ id: "urgent", identifier: "ENG-2", priority: 1 }),
      makeIssue({ id: "low", identifier: "ENG-3", priority: 4 }),
    ];
    const asc = sortIssues(issues, { ...DEFAULT_DISPLAY, ordering: "priority" });
    expect(ids(asc)).toEqual(["urgent", "low", "none"]);
    const desc = sortIssues(issues, {
      ...DEFAULT_DISPLAY,
      ordering: "priority",
      orderDirection: "desc",
    });
    expect(ids(desc)).toEqual(["low", "urgent", "none"]);
  });

  it("Due date: leere Fälligkeit in beiden Richtungen zuletzt", () => {
    const issues = [
      makeIssue({ id: "empty", identifier: "ENG-1", dueDate: null }),
      makeIssue({ id: "early", identifier: "ENG-2", dueDate: "2026-01-01" }),
      makeIssue({ id: "late", identifier: "ENG-3", dueDate: "2026-12-01" }),
    ];
    const o = { ...DEFAULT_DISPLAY, ordering: "dueDate" as const };
    expect(ids(sortIssues(issues, o))).toEqual(["early", "late", "empty"]);
    expect(ids(sortIssues(issues, { ...o, orderDirection: "desc" }))).toEqual([
      "late",
      "early",
      "empty",
    ]);
  });

  it("Created ascending beginnt mit dem ältesten", () => {
    const issues = [
      makeIssue({ id: "new", identifier: "ENG-2", createdAt: "2026-05-01T00:00:00Z" }),
      makeIssue({ id: "old", identifier: "ENG-1", createdAt: "2026-01-01T00:00:00Z" }),
    ];
    const o = { ...DEFAULT_DISPLAY, ordering: "createdAt" as const };
    expect(ids(sortIssues(issues, o))).toEqual(["old", "new"]);
    expect(ids(sortIssues(issues, { ...o, orderDirection: "desc" }))).toEqual(["new", "old"]);
    expect(defaultDirection("createdAt")).toBe("desc");
    expect(defaultDirection("dueDate")).toBe("asc");
  });

  it("Titel natürlich, Tie-Breaker createdAt absteigend, Manual nicht umkehrbar", () => {
    const t = [
      makeIssue({ id: "a", identifier: "ENG-1", title: "Item 10" }),
      makeIssue({ id: "b", identifier: "ENG-2", title: "item 2" }),
    ];
    expect(ids(sortIssues(t, { ...DEFAULT_DISPLAY, ordering: "title" }))).toEqual(["b", "a"]);
    const tie = [
      makeIssue({ id: "x", identifier: "ENG-1", priority: 2, createdAt: "2026-01-01T00:00:00Z" }),
      makeIssue({ id: "y", identifier: "ENG-2", priority: 2, createdAt: "2026-02-01T00:00:00Z" }),
    ];
    expect(ids(sortIssues(tie, DEFAULT_DISPLAY))).toEqual(["y", "x"]);
    const m = [
      makeIssue({ id: "m2", identifier: "ENG-1", sortOrder: 2 }),
      makeIssue({ id: "m1", identifier: "ENG-2", sortOrder: 1 }),
    ];
    expect(
      ids(sortIssues(m, { ...DEFAULT_DISPLAY, ordering: "manual", orderDirection: "desc" })),
    ).toEqual(["m1", "m2"]);
  });

  it("Status: nach Kategorie, dann Position; Assignee: unassigned zuletzt (F.13)", () => {
    const issues = [
      makeIssue({ id: "done", identifier: "ENG-1", stateId: "s-done" }),
      makeIssue({ id: "todo", identifier: "ENG-2", stateId: "s-todo" }),
      makeIssue({ id: "prog", identifier: "ENG-3", stateId: "s-progress" }),
    ];
    const byStatus = sortIssues(issues, { ...DEFAULT_DISPLAY, ordering: "status" });
    expect(ids(byStatus)).toEqual(["todo", "prog", "done"]);

    const people = [
      makeIssue({ id: "none", identifier: "ENG-4", assigneeId: null }),
      makeIssue({ id: "bob", identifier: "ENG-5", assigneeId: "u-bob" }),
      makeIssue({ id: "alice", identifier: "ENG-6", assigneeId: "u-alice" }),
    ];
    const names: Record<string, string> = { "u-bob": "Bob", "u-alice": "Alice" };
    const o = { ...DEFAULT_DISPLAY, ordering: "assignee" as const };
    const ctx = { assignee: (id: string | null) => (id ? (names[id] ?? null) : null) };
    expect(ids(sortIssues(people, o, ctx))).toEqual(["alice", "bob", "none"]);
    expect(ids(sortIssues(people, { ...o, orderDirection: "desc" }, ctx))).toEqual([
      "bob",
      "alice",
      "none",
    ]);
  });

  it("Estimate (leer zuletzt), Link count und Time in status", () => {
    const e = [
      makeIssue({ id: "e0", identifier: "ENG-1", estimate: null }),
      makeIssue({ id: "e5", identifier: "ENG-2", estimate: 5 }),
      makeIssue({ id: "e2", identifier: "ENG-3", estimate: 2 }),
    ];
    const o = { ...DEFAULT_DISPLAY, ordering: "estimate" as const };
    expect(ids(sortIssues(e, o))).toEqual(["e2", "e5", "e0"]);
    expect(ids(sortIssues(e, { ...o, orderDirection: "desc" }))).toEqual(["e5", "e2", "e0"]);

    const l = [
      makeIssue({ id: "l1", identifier: "ENG-4", linkCount: 1 }),
      makeIssue({ id: "l3", identifier: "ENG-5", linkCount: 3 }),
    ];
    expect(
      ids(sortIssues(l, { ...DEFAULT_DISPLAY, ordering: "linkCount", orderDirection: "desc" })),
    ).toEqual(["l3", "l1"]);
    expect(defaultDirection("linkCount")).toBe("desc");

    const t = [
      makeIssue({ id: "recent", identifier: "ENG-6", stateEnteredAt: "2026-10-01T00:00:00Z" }),
      makeIssue({ id: "stale", identifier: "ENG-7", stateEnteredAt: "2026-01-01T00:00:00Z" }),
    ];
    expect(ids(sortIssues(t, { ...DEFAULT_DISPLAY, ordering: "timeInStatus" }))).toEqual([
      "stale",
      "recent",
    ]);
  });
});
