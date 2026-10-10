import { describe, expect, it } from "vitest";
import { safeRedirect } from "./auth";

describe("safeRedirect", () => {
  it.each([
    ["/acme/inbox", "/acme/inbox"],
    ["/acme/issue/ENG-1?x=1#c", "/acme/issue/ENG-1?x=1#c"],
  ])("erlaubt interne Pfade %s", (input, expected) => {
    expect(safeRedirect(input)).toBe(expected);
  });

  it.each([
    "https://evil.example",
    "//evil.example",
    "/\\evil.example",
    "/\t/evil.example",
    "javascript:alert(1)",
    "",
    undefined,
    42,
  ])("lehnt %s ab", (input) => {
    expect(safeRedirect(input)).toBe("/");
  });
});
