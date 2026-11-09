import { describe, expect, it } from "vitest";

import { milestoneDropSortOrder } from "./milestone-reorder";

// Meilensteine in Anzeige-Reihenfolge (sortOrder aufsteigend).
const MS = [
  { id: "m1", sortOrder: 1 },
  { id: "m2", sortOrder: 2 },
  { id: "m3", sortOrder: 3 },
];

describe("milestoneDropSortOrder (LIN-47)", () => {
  it("vor den ersten ziehen: kleiner als der bisherige Erste", () => {
    // m3 auf m1 fallen lassen → neue Reihenfolge m3, m1, m2.
    expect(milestoneDropSortOrder(MS, "m3", "m1")).toBe(0); // between(undefined, 1) = 0
  });

  it("ans Ende ziehen: größer als der bisherige Letzte", () => {
    // m1 auf m3 fallen lassen → m2, m3, m1.
    expect(milestoneDropSortOrder(MS, "m1", "m3")).toBe(4); // between(3, undefined) = 4
  });

  it("zwischen zwei ziehen: Mittelwert der neuen Nachbarn", () => {
    // m1 auf m2 fallen lassen → m2, m1, m3.
    expect(milestoneDropSortOrder(MS, "m1", "m2")).toBe(2.5); // between(2, 3)
  });

  it("kein Ziel oder gleiche Position: nichts zu tun", () => {
    expect(milestoneDropSortOrder(MS, "m2", null)).toBeNull();
    expect(milestoneDropSortOrder(MS, "m2", "m2")).toBeNull();
    expect(milestoneDropSortOrder(MS, "x", "m1")).toBeNull();
  });
});
