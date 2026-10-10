import { describe, expect, it } from "vitest";

import { buildFilter, chipToFilter, type FilterChip } from "./filter-model";

const chip = (
  over: Partial<FilterChip> & Pick<FilterChip, "field" | "op" | "values">,
): FilterChip => ({
  id: "c",
  ...over,
});

describe("chipToFilter (docs/05 §1.6)", () => {
  it("assignee is / is not / any of", () => {
    expect(chipToFilter(chip({ field: "assignee", op: "is", values: ["u-bob"] }))).toEqual({
      assignee: { id: { in: ["u-bob"] } },
    });
    expect(chipToFilter(chip({ field: "assignee", op: "isNot", values: ["u-bob"] }))).toEqual({
      assignee: { id: { nin: ["u-bob"] } },
    });
    expect(
      chipToFilter(chip({ field: "assignee", op: "is", values: ["u-bob", "u-carol"] })),
    ).toEqual({ assignee: { id: { in: ["u-bob", "u-carol"] } } });
  });

  it("assignee special values: none, me, none or Bob", () => {
    expect(chipToFilter(chip({ field: "assignee", op: "is", values: ["none"] }))).toEqual({
      assignee: { null: true },
    });
    expect(chipToFilter(chip({ field: "assignee", op: "isNot", values: ["none"] }))).toEqual({
      assignee: { null: false },
    });
    expect(chipToFilter(chip({ field: "assignee", op: "is", values: ["me"] }))).toEqual({
      assignee: { isMe: { eq: true } },
    });
    expect(chipToFilter(chip({ field: "assignee", op: "is", values: ["none", "u-bob"] }))).toEqual({
      assignee: { or: [{ id: { in: ["u-bob"] } }, { null: true }] },
    });
  });

  it("labels: include, include all, exclude, no labels", () => {
    expect(chipToFilter(chip({ field: "label", op: "includeAny", values: ["l-bug"] }))).toEqual({
      labels: { some: { id: { in: ["l-bug"] } } },
    });
    expect(chipToFilter(chip({ field: "label", op: "excludeAny", values: ["l-bug"] }))).toEqual({
      labels: { every: { id: { nin: ["l-bug"] } } },
    });
    expect(
      chipToFilter(chip({ field: "label", op: "includeAll", values: ["l-bug", "l-fe"] })),
    ).toEqual({
      and: [
        { labels: { some: { id: { in: ["l-bug"] } } } },
        { labels: { some: { id: { in: ["l-fe"] } } } },
      ],
    });
    expect(chipToFilter(chip({ field: "label", op: "includeAny", values: ["none"] }))).toEqual({
      labels: { length: { eq: 0 } },
    });
  });

  it("priority uses sets of numbers", () => {
    expect(chipToFilter(chip({ field: "priority", op: "is", values: ["1", "2"] }))).toEqual({
      priority: { in: [1, 2] },
    });
    expect(chipToFilter(chip({ field: "priority", op: "isNot", values: ["0"] }))).toEqual({
      priority: { nin: [0] },
    });
  });

  it("status type, project and cycle special values", () => {
    expect(chipToFilter(chip({ field: "stateType", op: "is", values: ["started"] }))).toEqual({
      state: { type: { in: ["started"] } },
    });
    expect(chipToFilter(chip({ field: "project", op: "is", values: ["none"] }))).toEqual({
      project: { null: true },
    });
    expect(chipToFilter(chip({ field: "cycle", op: "is", values: ["current"] }))).toEqual({
      cycle: { isActive: { eq: true } },
    });
    expect(chipToFilter(chip({ field: "cycle", op: "is", values: ["upcoming"] }))).toEqual({
      cycle: { isNext: { eq: true } },
    });
  });

  it("overdue adds the open-status condition; relative and absolute dates", () => {
    expect(chipToFilter(chip({ field: "dueDate", op: "is", values: ["overdue"] }))).toEqual({
      dueDate: { lt: "P0D" },
      state: { type: { nin: ["completed", "canceled", "duplicate"] } },
    });
    expect(chipToFilter(chip({ field: "dueDate", op: "isNot", values: ["overdue"] }))).toEqual({
      or: [{ dueDate: { gte: "P0D" } }, { dueDate: { null: true } }],
    });
    expect(
      chipToFilter(chip({ field: "createdAt", op: "before", values: ["2026-09-01"] })),
    ).toEqual({
      createdAt: { lt: "2026-09-01" },
    });
    expect(chipToFilter(chip({ field: "createdAt", op: "after", values: ["-P1W"] }))).toEqual({
      createdAt: { gt: "-P1W" },
    });
  });

  it("team incl. sub-teams uses ancestors", () => {
    expect(chipToFilter(chip({ field: "team", op: "is", values: ["t-eng"] }))).toEqual({
      team: { id: { in: ["t-eng"] } },
    });
    expect(
      chipToFilter(chip({ field: "team", op: "is", values: ["t-eng"], subTeams: true })),
    ).toEqual({
      team: { or: [{ id: { in: ["t-eng"] } }, { ancestors: { some: { id: { in: ["t-eng"] } } } }] },
    });
  });
});

describe("buildFilter", () => {
  it("empty, single and multiple chips; incomplete chips are ignored", () => {
    expect(buildFilter([])).toEqual({});
    const a = chip({ id: "a", field: "assignee", op: "is", values: ["me"] });
    const p = chip({ id: "p", field: "priority", op: "is", values: ["1"] });
    expect(buildFilter([a])).toEqual({ assignee: { isMe: { eq: true } } });
    expect(buildFilter([a, p])).toEqual({
      and: [{ assignee: { isMe: { eq: true } } }, { priority: { in: [1] } }],
    });
    expect(buildFilter([a, chip({ field: "label", op: "includeAny", values: [] })])).toEqual({
      assignee: { isMe: { eq: true } },
    });
  });
});
