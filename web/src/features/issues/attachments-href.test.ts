import { describe, expect, it } from "vitest";

import { safeHref } from "./attachments";

describe("safeHref", () => {
  it("lässt nur http(s) zu", () => {
    expect(safeHref("https://example.com/a")).toBe("https://example.com/a");
    expect(safeHref("http://example.com/")).toBe("http://example.com/");
    expect(safeHref("javascript:alert(1)")).toBeUndefined();
    expect(safeHref(" JaVaScRiPt:alert(1)")).toBeUndefined();
    expect(safeHref("data:text/html,<script>alert(1)</script>")).toBeUndefined();
    expect(safeHref("vbscript:x")).toBeUndefined();
  });
});
