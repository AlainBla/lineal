import { describe, expect, it } from "vitest";

import { statusDropPosition } from "./status-reorder";

// Statuslisten in Anzeige-Reihenfolge (position aufsteigend); zwei je Kategorie zum Sortieren.
const STATES = [
  { id: "t1", type: "unstarted", position: 0 },
  { id: "t2", type: "unstarted", position: 1 },
  { id: "p1", type: "started", position: 2 },
  { id: "p2", type: "started", position: 3 },
  { id: "p3", type: "started", position: 4 },
];

describe("statusDropPosition (LIN-73)", () => {
  it("nach oben in derselben Kategorie: Mittelwert der neuen Nachbarn", () => {
    // p3 auf p1 fallen lassen → p3, p1, p2.
    expect(statusDropPosition(STATES, "p3", "p1")).toBe(1); // between(undefined, 2) = 1
    // p3 auf p2 fallen lassen → p1, p3, p2.
    expect(statusDropPosition(STATES, "p3", "p2")).toBe(2.5); // between(2, 3)
  });

  it("ans Ende der Kategorie ziehen: größer als der bisherige Letzte", () => {
    // p1 auf p3 fallen lassen → p2, p3, p1.
    expect(statusDropPosition(STATES, "p1", "p3")).toBe(5); // between(4, undefined) = 5
  });

  it("an den Anfang der Kategorie ziehen: kleiner als der bisherige Erste", () => {
    // t2 auf t1 fallen lassen → t2, t1.
    expect(statusDropPosition(STATES, "t2", "t1")).toBe(-1); // between(undefined, 0) = -1
  });

  it("Ziel in anderer Kategorie: nichts zu tun (kategorie-intern)", () => {
    expect(statusDropPosition(STATES, "t1", "p1")).toBeNull();
    expect(statusDropPosition(STATES, "p1", "t1")).toBeNull();
  });

  it("kein Ziel, gleiche Position oder unbekannt: nichts zu tun", () => {
    expect(statusDropPosition(STATES, "p1", null)).toBeNull();
    expect(statusDropPosition(STATES, "p1", "p1")).toBeNull();
    expect(statusDropPosition(STATES, "x", "p1")).toBeNull();
    expect(statusDropPosition(STATES, "p1", "x")).toBeNull();
  });
});
