import { describe, expect, it } from "vitest";

import { teamScopeIds } from "./use-filter-sources";

const LIN = "lin";
const LIN_SUB = "lin-sub";
const LIN_SUB2 = "lin-sub-2"; // Enkel: Unter-Team des Unter-Teams
const KOL = "kol";

const teams = [
  { id: LIN, parentId: null },
  { id: KOL, parentId: null },
  { id: LIN_SUB, parentId: LIN },
  { id: LIN_SUB2, parentId: LIN_SUB },
  { id: "kol-sub", parentId: KOL },
];

describe("teamScopeIds", () => {
  it("ohne Team gewählt: kein Scope (null)", () => {
    expect(teamScopeIds(undefined, teams)).toBeNull();
  });

  it("scopt auf das Team und alle Nachfahren, nicht auf fremde Teams", () => {
    const scope = teamScopeIds(LIN, teams)!;
    expect(scope.has(LIN)).toBe(true);
    expect(scope.has(LIN_SUB)).toBe(true);
    expect(scope.has(LIN_SUB2)).toBe(true); // transitiv über die parentId-Kette
    expect(scope.has(KOL)).toBe(false); // Geschwister-Team bleibt draußen
    expect(scope.has("kol-sub")).toBe(false);
  });

  it("Blatt-Team ohne Unter-Teams: nur es selbst", () => {
    expect([...teamScopeIds(KOL, [{ id: KOL, parentId: null }])!]).toEqual([KOL]);
  });
});
