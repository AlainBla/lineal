import { describe, expect, it } from "vitest";

import {
  conditionsToFilter,
  describeConditions,
  describeDuration,
  describeRhythm,
  durationToIso,
  filterToConditions,
  minuteToTime,
  parseDuration,
  relativeDays,
  relativeDaysToIso,
  relativeDaysValid,
  timeToMinute,
} from "./team-extra-model";

describe("SLA conditions", () => {
  it("builds the filter from priorities and labels", () => {
    expect(conditionsToFilter({ priorities: [], labelIds: [] })).toEqual({});
    expect(conditionsToFilter({ priorities: [1], labelIds: [] })).toEqual({
      priority: { eq: 1 },
    });
    expect(conditionsToFilter({ priorities: [2, 1, 2], labelIds: ["a", "b"] })).toEqual({
      priority: { in: [1, 2] },
      labels: { some: { id: { in: ["a", "b"] } } },
    });
  });

  it("round-trips its own filters and rejects anything else", () => {
    for (const c of [
      { priorities: [], labelIds: [] },
      { priorities: [1], labelIds: [] },
      { priorities: [3, 4], labelIds: ["x"] },
    ])
      expect(filterToConditions(conditionsToFilter(c))).toEqual(c);
    expect(filterToConditions({ assignee: { null: true } })).toBeNull();
    expect(filterToConditions({ priority: { gte: 2 } })).toBeNull();
    expect(filterToConditions({ labels: { every: {} } })).toBeNull();
    expect(filterToConditions({ and: [] })).toBeNull();
    expect(filterToConditions("x")).toBeNull();
  });

  it("describes conditions in words", () => {
    const name = (id: string) => `L${id}`;
    expect(describeConditions({}, name)).toBe("All issues");
    expect(describeConditions({ priority: { eq: 1 } }, name)).toBe("Priority Urgent");
    expect(describeConditions(conditionsToFilter({ priorities: [2], labelIds: ["7"] }), name)).toBe(
      "Priority High and Label L7",
    );
    expect(describeConditions({ project: { null: true } }, name)).toBe("Custom conditions");
  });
});

describe("SLA durations", () => {
  it("builds and parses ISO durations", () => {
    expect(durationToIso(36, "hours")).toBe("PT36H");
    expect(durationToIso(3, "days")).toBe("P3D");
    expect(durationToIso(2.9, "weeks")).toBe("P2W");
    expect(durationToIso(0, "days")).toBe("P1D");
    expect(parseDuration("PT12H")).toEqual({ amount: 12, unit: "hours" });
    expect(parseDuration("P2W")).toEqual({ amount: 2, unit: "weeks" });
    expect(parseDuration("P1DT2H")).toBeNull();
    expect(parseDuration(null)).toBeNull();
  });

  it("describes durations for calendar time and business days", () => {
    expect(describeDuration("PT24H", false)).toBe("24 hours");
    expect(describeDuration("P3D", false)).toBe("3 days");
    expect(describeDuration("PT36H", false)).toBe("36 hours");
    expect(describeDuration("P3D", true)).toBe("3 business days");
    expect(describeDuration("P1D", true)).toBe("1 business day");
    expect(describeDuration("P2W", true)).toBe("10 business days");
    expect(describeDuration(null, false)).toBe("–");
  });
});

describe("Recurring helpers", () => {
  it("converts times of day", () => {
    expect(timeToMinute("08:30")).toBe(510);
    expect(timeToMinute("0:05")).toBe(5);
    expect(timeToMinute("24:00")).toBeNull();
    expect(timeToMinute("12:60")).toBeNull();
    expect(timeToMinute("")).toBeNull();
    expect(minuteToTime(510)).toBe("08:30");
    expect(minuteToTime(0)).toBe("00:00");
  });

  it("describes rhythms", () => {
    expect(describeRhythm({ frequency: "daily", interval: 1, minuteOfDay: 480 })).toBe(
      "Every day at 08:00",
    );
    expect(describeRhythm({ frequency: "weekly", interval: 2, weekday: 5, minuteOfDay: 0 })).toBe(
      "Every 2 weeks on Friday at 00:00",
    );
    expect(
      describeRhythm({ frequency: "monthly", interval: 1, dayOfMonth: 22, minuteOfDay: 60 }),
    ).toBe("Every month on the 22nd at 01:00");
    expect(
      describeRhythm({ frequency: "monthly", interval: 3, dayOfMonth: 11, minuteOfDay: 60 }),
    ).toBe("Every 3 months on the 11th at 01:00");
  });

  it("handles relative days of templates", () => {
    expect(relativeDays("P3D")).toBe("3");
    expect(relativeDays("P2W")).toBe("14");
    expect(relativeDays("P1W3D")).toBe("10");
    expect(relativeDays("P")).toBe("");
    expect(relativeDays(undefined)).toBe("");
    expect(relativeDaysToIso("14")).toBe("P14D");
    expect(relativeDaysToIso("")).toBeUndefined();
    expect(relativeDaysValid("")).toBe(true);
    expect(relativeDaysValid("0")).toBe(true);
    expect(relativeDaysValid("-1")).toBe(false);
    expect(relativeDaysValid("1.5")).toBe(false);
    expect(relativeDaysValid("99999")).toBe(false);
  });
});
