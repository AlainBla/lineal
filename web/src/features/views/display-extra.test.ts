import { describe, expect, it } from "vitest";

import { makeIssue } from "@/features/issues/test-fixtures";

import {
  applySubGrouping,
  DEFAULT_DISPLAY,
  flatIssues,
  groupIssues,
  isCustomDisplay,
  mergeDisplay,
  nestIssues,
  withEmptyGroups,
} from "./display-options";
import { buildFilter, chipToFilter, type FilterChip } from "./filter-model";
import { filterToChips } from "./filter-parse";

const names = {
  assignee: (id: string | null) => (id === "u-bob" ? "Bob" : id),
  project: () => null,
  team: () => null,
  cycle: () => null,
  milestone: (id: string | null) => (id === "m-1" ? "Beta" : null),
};

describe("Anzeigeoptionen: Untergruppierung, Verschachtelung, Display properties", () => {
  it("mergeDisplay übernimmt gültige Werte und verwirft ungültige", () => {
    const o = mergeDisplay({
      subGrouping: "assignee",
      grouping: "assignee",
      nestedSubIssues: "showMatching",
      displayProperties: ["id", "labels", "bogus"],
    });
    expect(o.subGrouping).toBe("none"); // gleich der Gruppierung
    expect(o.nestedSubIssues).toBe("showMatching");
    expect(o.displayProperties).toEqual(["id", "labels"]);
    expect(isCustomDisplay(DEFAULT_DISPLAY)).toBe(false);
    expect(isCustomDisplay(o)).toBe(true);
  });

  it("gruppiert nach Meilenstein und unterteilt jede Gruppe", () => {
    const issues = [
      makeIssue({ id: "a", identifier: "ENG-a", projectMilestoneId: "m-1", assigneeId: "u-bob" }),
      makeIssue({ id: "b", identifier: "ENG-b", projectMilestoneId: "m-1", assigneeId: null }),
      makeIssue({ id: "c", identifier: "ENG-c", projectMilestoneId: null, assigneeId: "u-bob" }),
    ];
    const groups = applySubGrouping(groupIssues(issues, "milestone", names), "assignee", names);
    expect(groups.map((g) => g.label)).toEqual(["Beta", "No milestone"]);
    expect(groups[0]!.subGroups!.map((g) => g.label)).toEqual(["Bob", "No assignee"]);
    expect(flatIssues(groups).map((i) => i.id)).toEqual(["a", "b", "c"]);
  });

  it("Show empty groups ergänzt leere Gruppen in Quellreihenfolge", () => {
    const groups = groupIssues(
      [makeIssue({ id: "a", identifier: "ENG-a", projectMilestoneId: "m-2" })],
      "milestone",
      {
        ...names,
        milestone: () => "Later",
      },
    );
    const all = [
      { key: "m-1", label: "Beta" },
      { key: "m-2", label: "Later" },
      { key: "none", label: "No milestone" },
    ];
    const withEmpty = withEmptyGroups(groups, all, "milestone");
    expect(withEmpty.map((g) => [g.key, g.issues.length])).toEqual([
      ["m-1", 0],
      ["m-2", 1],
      ["none", 0],
    ]);
  });

  it("nestIssues rückt Sub-Issues unter ihren Parent ein", () => {
    const issues = [
      makeIssue({ id: "c1", identifier: "ENG-c1", parentId: "p" }),
      makeIssue({ id: "p", identifier: "ENG-p" }),
      makeIssue({ id: "x", identifier: "ENG-x", parentId: "elsewhere" }),
    ];
    expect(nestIssues(issues).map((r) => [r.issue.id, r.depth])).toEqual([
      ["p", 0],
      ["c1", 1],
      ["x", 0],
    ]);
  });
});

describe("Neue Filterkategorien (docs/05 §1.5)", () => {
  const chip = (c: Omit<FilterChip, "id">): FilterChip => ({ id: "c", ...c });
  const cases: Array<[Omit<FilterChip, "id">, unknown]> = [
    [{ field: "state", op: "is", values: ["s-1"] }, { state: { id: { in: ["s-1"] } } }],
    [
      { field: "milestone", op: "is", values: ["m-1"] },
      { projectMilestone: { id: { in: ["m-1"] } } },
    ],
    [{ field: "milestone", op: "is", values: ["none"] }, { projectMilestone: { null: true } }],
    [
      { field: "relations", op: "is", values: ["blocked"] },
      { hasBlockedByRelations: { eq: true } },
    ],
    [
      { field: "addedToCycle", op: "is", values: ["during"] },
      { addedToCyclePeriod: { in: ["during"] } },
    ],
    [{ field: "autoClosed", op: "is", values: ["yes"] }, { autoClosedAt: { null: false } }],
    [
      { field: "content", op: "contains", values: ["login"] },
      { searchableContent: { contains: "login" } },
    ],
    [
      { field: "links", op: "contains", values: ["github.com"] },
      { attachments: { some: { url: { containsIgnoreCase: "github.com" } } } },
    ],
  ];

  it.each(cases)("%j → Filter-JSON und zurück zum Chip", (c, json) => {
    expect(chipToFilter(chip(c))).toEqual(json);
    const parsed = filterToChips(json);
    expect(parsed.rest).toEqual([]);
    expect(parsed.chips.map(({ field, op, values }) => ({ field, op, values }))).toEqual([c]);
  });

  it("mehrere Relationen werden mit oder verknüpft", () => {
    expect(
      buildFilter([chip({ field: "relations", op: "is", values: ["blocked", "blocking"] })]),
    ).toEqual({
      or: [{ hasBlockedByRelations: { eq: true } }, { hasBlockingRelations: { eq: true } }],
    });
  });
});
