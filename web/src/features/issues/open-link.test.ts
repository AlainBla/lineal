import { afterEach, describe, expect, it, vi } from "vitest";

import { linkTarget, openLink } from "./open-link";

afterEach(() => vi.restoreAllMocks());

describe("openLink", () => {
  it("navigiert interne Pfade im Router und öffnet fremde in neuem Tab", () => {
    const navigate = vi.fn();
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    expect(openLink(`${window.location.origin}/ws/issue/KOL-1`, { navigate })).toBe(true);
    expect(navigate).toHaveBeenCalledWith({ to: "/ws/issue/KOL-1" });
    expect(openLink("https://example.com/a", { navigate })).toBe(true);
    expect(open).toHaveBeenCalledWith("https://example.com/a", "_blank", "noopener,noreferrer");
    expect(openLink("/ws/issue/KOL-2", { navigate }, { newTab: true })).toBe(true);
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("ignoriert unsichere Schemata", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    expect(openLink("javascript:alert(1)", { navigate: vi.fn() })).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("findet das Linkziel auch in Chips", () => {
    const a = document.createElement("span");
    a.dataset.href = "/ws/issue/KOL-3";
    const inner = document.createElement("b");
    a.appendChild(inner);
    expect(linkTarget(inner)).toBe("/ws/issue/KOL-3");
    expect(linkTarget(document.createElement("i"))).toBeNull();
  });
});
