import { describe, expect, it } from "vitest";

import { dropKeyFromId } from "./milestone-drop";

describe("dropKeyFromId (LIN-46)", () => {
  it("löst Listengruppen- und Panel-Ablage-IDs auf denselben Meilenstein-Schlüssel auf", () => {
    expect(dropKeyFromId("group-drop:m-1")).toBe("m-1");
    expect(dropKeyFromId("panel-drop:m-1")).toBe("m-1");
    expect(dropKeyFromId("group-drop:none")).toBe("none");
    expect(dropKeyFromId("panel-drop:none")).toBe("none");
  });

  it("ignoriert fremde oder fehlende IDs", () => {
    expect(dropKeyFromId("abc")).toBeNull();
    expect(dropKeyFromId(null)).toBeNull();
    expect(dropKeyFromId(undefined)).toBeNull();
    expect(dropKeyFromId("")).toBeNull();
  });
});
