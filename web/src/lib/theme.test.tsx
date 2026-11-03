import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import "@/app/test-polyfills";
import {
  applyTheme,
  DEFAULT_THEME,
  getStoredTheme,
  getTheme,
  resetThemeState,
  resolveTheme,
  setTheme,
  THEME_STORAGE_KEY,
  toggledTheme,
  useTheme,
  watchSystemTheme,
} from "./theme";

function Probe() {
  const { preference, resolved, setTheme: set } = useTheme();
  return (
    <div>
      <span data-testid="state">{`${preference}/${resolved}`}</span>
      <button onClick={() => set(toggledTheme(preference))}>toggle</button>
    </div>
  );
}

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    resetThemeState();
    document.documentElement.className = "";
  });
  afterEach(() => localStorage.clear());

  it("defaults to system", () => {
    expect(DEFAULT_THEME).toBe("system");
    expect(getStoredTheme()).toBe("system");
    expect(getTheme()).toBe("system");
  });

  it("keeps existing stored values", () => {
    for (const v of ["dark", "light", "system"] as const) {
      localStorage.setItem(THEME_STORAGE_KEY, v);
      expect(getStoredTheme()).toBe(v);
    }
  });

  it("follows OS changes live while `system` is selected", () => {
    let dark = false;
    let handler: (() => void) | undefined;
    const original = window.matchMedia;
    window.matchMedia = ((q: string) => ({
      matches: dark,
      media: q,
      addEventListener: (_: string, h: () => void) => (handler = h),
      removeEventListener: () => (handler = undefined),
    })) as unknown as typeof window.matchMedia;
    try {
      const stop = watchSystemTheme();
      expect(getTheme()).toBe("system");
      dark = true;
      handler?.();
      expect(document.documentElement).toHaveClass("dark");
      dark = false;
      handler?.();
      expect(document.documentElement).not.toHaveClass("dark");
      setTheme("dark");
      dark = false;
      handler?.();
      expect(document.documentElement).toHaveClass("dark"); // explizite Wahl bleibt
      stop();
      expect(handler).toBeUndefined();
    } finally {
      window.matchMedia = original;
    }
  });

  it("ignores garbage in storage", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    expect(getStoredTheme()).toBe("system");
  });

  it("applies the dark class and color-scheme to <html>", () => {
    applyTheme("dark");
    expect(document.documentElement).toHaveClass("dark");
    expect(document.documentElement.style.colorScheme).toBe("dark");
    applyTheme("light");
    expect(document.documentElement).not.toHaveClass("dark");
    expect(document.documentElement.style.colorScheme).toBe("light");
  });

  it("resolves `system` through prefers-color-scheme", () => {
    expect(resolveTheme("system")).toBe("light"); // Polyfill meldet matches=false
    expect(resolveTheme("dark")).toBe("dark");
  });

  it("switches theme, persists it and updates subscribers", async () => {
    const user = userEvent.setup();
    render(<Probe />);
    expect(screen.getByTestId("state")).toHaveTextContent("system/light");

    await user.click(screen.getByRole("button", { name: "toggle" }));
    expect(screen.getByTestId("state")).toHaveTextContent("dark/dark");
    expect(document.documentElement).toHaveClass("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");

    await user.click(screen.getByRole("button", { name: "toggle" }));
    expect(screen.getByTestId("state")).toHaveTextContent("light/light");
    expect(document.documentElement).not.toHaveClass("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
  });
});
