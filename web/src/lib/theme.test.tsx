import { act, render, screen } from "@testing-library/react";
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

  it("defaults to dark", () => {
    expect(DEFAULT_THEME).toBe("dark");
    expect(getStoredTheme()).toBe("dark");
    expect(getTheme()).toBe("dark");
  });

  it("ignores garbage in storage", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    expect(getStoredTheme()).toBe("dark");
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
    expect(screen.getByTestId("state")).toHaveTextContent("dark/dark");

    await user.click(screen.getByRole("button", { name: "toggle" }));
    expect(screen.getByTestId("state")).toHaveTextContent("light/light");
    expect(document.documentElement).not.toHaveClass("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");

    act(() => setTheme("dark"));
    expect(screen.getByTestId("state")).toHaveTextContent("dark/dark");
    expect(document.documentElement).toHaveClass("dark");
  });
});
