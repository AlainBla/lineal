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
});
