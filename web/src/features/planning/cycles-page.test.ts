import { describe, expect, it } from "vitest";

import { formatCapacity } from "./cycles-page";

describe("formatCapacity", () => {
  it("zeigt ganze Zahlen ohne Nachkommastellen", () => {
    expect(formatCapacity(8)).toBe("8");
  });
  it("rundet Bruchteile auf eine Stelle", () => {
    expect(formatCapacity(7.666)).toBe("7.7");
  });
});
