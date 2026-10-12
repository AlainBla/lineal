import { describe, expect, it } from "vitest";

import { buildFilter, chipToFilter, type FilterChip } from "./filter-model";
import { combineFilter, filterToChips } from "./filter-parse";

const c = (
  over: Pick<FilterChip, "field" | "op" | "values"> & Partial<FilterChip>,
): FilterChip => ({ id: "c", ...over });

const strip = (chips: FilterChip[]) =>
  chips.map((x) => {
    const o: Partial<FilterChip> = { ...x };
    delete o.id;
    return o;
  });

describe("filterToChips: Rundreise Chip → JSON → Chip", () => {
  const cases: FilterChip[] = [
    c({ field: "assignee", op: "is", values: ["u-bob"] }),
    c({ field: "assignee", op: "isNot", values: ["u-bob", "u-carol"] }),
    c({ field: "assignee", op: "is", values: ["me"] }),
    c({ field: "assignee", op: "is", values: ["none"] }),
    c({ field: "assignee", op: "isNot", values: ["none"] }),
    c({ field: "assignee", op: "is", values: ["u-bob", "none"] }),
    c({ field: "creator", op: "is", values: ["u-bob", "me"] }),
    c({ field: "stateType", op: "is", values: ["started", "unstarted"] }),
    c({ field: "stateType", op: "isNot", values: ["completed"] }),
    c({ field: "priority", op: "is", values: ["1", "2"] }),
    c({ field: "priority", op: "isNot", values: ["0"] }),
    c({ field: "label", op: "includeAny", values: ["l-bug", "l-fe"] }),
    c({ field: "label", op: "includeAll", values: ["l-bug", "l-fe"] }),
    c({ field: "label", op: "excludeAny", values: ["l-bug"] }),
    c({ field: "label", op: "includeAny", values: ["none"] }),
    c({ field: "label", op: "excludeAny", values: ["none"] }),
    c({ field: "project", op: "is", values: ["p-1"] }),
    c({ field: "project", op: "is", values: ["none"] }),
    c({ field: "project", op: "isNot", values: ["p-1"] }),
    c({ field: "cycle", op: "is", values: ["current", "upcoming"] }),
    c({ field: "cycle", op: "is", values: ["none"] }),
    c({ field: "team", op: "is", values: ["t-eng"] }),
    c({ field: "team", op: "is", values: ["t-eng"], subTeams: true }),
    c({ field: "team", op: "isNot", values: ["t-eng"] }),
    c({ field: "dueDate", op: "is", values: ["overdue"] }),
    c({ field: "dueDate", op: "isNot", values: ["overdue"] }),
    c({ field: "dueDate", op: "before", values: ["2026-09-01"] }),
    c({ field: "dueDate", op: "after", values: ["2026-09-01"] }),
    c({ field: "createdAt", op: "after", values: ["-P1W"] }),
    c({ field: "updatedAt", op: "before", values: ["2026-09-01"] }),
    c({ field: "startedAt", op: "after", values: ["2026-01-01"] }),
    c({ field: "completedAt", op: "before", values: ["2026-09-30"] }),
    c({ field: "canceledAt", op: "after", values: ["-P2W"] }),
    c({ field: "estimate", op: "is", values: ["3"] }),
    c({ field: "estimate", op: "isNot", values: ["3"] }),
    c({ field: "estimate", op: "lessThan", values: ["5"] }),
    c({ field: "estimate", op: "greaterThan", values: ["1"] }),
    c({ field: "subscribers", op: "includeAny", values: ["u-bob"] }),
    c({ field: "subscribers", op: "excludeAny", values: ["u-bob", "u-carol"] }),
    c({ field: "parent", op: "is", values: ["none"] }),
    c({ field: "parent", op: "isNot", values: ["none"] }),
    c({ field: "slaStatus", op: "is", values: ["breached", "highRisk"] }),
    c({ field: "title", op: "contains", values: ["login"] }),
  ];

  it.each(cases.map((x) => [`${x.field} ${x.op} ${x.values.join(",")}`, x] as const))(
    "%s",
    (_name, chip) => {
      const parsed = filterToChips(chipToFilter(chip));
      expect(parsed.rest).toEqual([]);
      expect(strip(parsed.chips)).toEqual(strip([chip]));
    },
  );

  it("mehrere Chips (and) bleiben getrennte Chips", () => {
    const chips = [cases[0]!, cases[7]!, cases[11]!, cases[24]!];
    const parsed = filterToChips(buildFilter(chips));
    expect(parsed.rest).toEqual([]);
    expect(strip(parsed.chips)).toEqual(strip(chips));
  });
});

describe("filterToChips: Server-Normalform und Dokumentbeispiele (docs/05 §1.6)", () => {
  it("eq/neq werden wie einelementiges in/nin gelesen", () => {
    const p = filterToChips({ assignee: { id: { eq: "u-bob" } }, priority: { eq: 0 } });
    expect(p.rest).toEqual([]);
    expect(strip(p.chips)).toEqual([
      { field: "assignee", op: "is", values: ["u-bob"] },
      { field: "priority", op: "is", values: ["0"] },
    ]);
  });

  it("Reihenfolge im or ist egal (null zuerst)", () => {
    const p = filterToChips({
      assignee: { or: [{ null: true }, { id: { in: ["u-bob"] } }] },
    });
    expect(p.rest).toEqual([]);
    expect(strip(p.chips)).toEqual([{ field: "assignee", op: "is", values: ["none", "u-bob"] }]);
  });

  it("leerer Filter ergibt nichts", () => {
    expect(filterToChips({})).toEqual({ chips: [], rest: [] });
    expect(filterToChips(null)).toEqual({ chips: [], rest: [] });
  });
});

describe("filterToChips: erweiterter Filter statt Datenverlust", () => {
  it("or auf oberster Ebene bleibt unverändert im Rest", () => {
    const or = { or: [{ priority: { in: [1] } }, { assignee: { isMe: { eq: true } } }] };
    const p = filterToChips({ and: [{ title: { containsIgnoreCaseAndAccent: "x" } }, or] });
    expect(strip(p.chips)).toEqual([{ field: "title", op: "contains", values: ["x"] }]);
    expect(p.rest).toEqual([or]);
  });

  it("unbekannte Felder und Operatoren landen im Rest", () => {
    const f1 = { delegate: { id: { in: ["u-1"] } } };
    const f2 = { title: { startsWithIgnoreCase: "Fix" } };
    const f3 = { dueDate: { lt: "P0D" } };
    const p = filterToChips({ and: [f1, f2, f3, { priority: { in: [2] } }] });
    expect(strip(p.chips)).toEqual([{ field: "priority", op: "is", values: ["2"] }]);
    expect(p.rest).toEqual([f1, f2, f3]);
  });

  it("Zusammensetzen verliert nichts: chips UND rest", () => {
    const rest = { or: [{ priority: { in: [1] } }, { estimate: { null: true } }] };
    const p = filterToChips({ and: [{ assignee: { isMe: { eq: true } } }, rest] });
    const rebuilt = combineFilter(buildFilter(p.chips), p.rest);
    expect(rebuilt).toEqual({ and: [{ assignee: { isMe: { eq: true } } }, rest] });
    expect(combineFilter({}, [])).toEqual({});
    expect(combineFilter({}, [rest])).toEqual(rest);
  });
});
