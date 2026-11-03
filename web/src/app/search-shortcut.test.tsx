import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { CommandRegistry, useActions } from "@/app/commands";
import { buildGlobalActions } from "@/app/global-actions";
import { attachShortcuts, ShortcutRegistry } from "@/lib/shortcuts";

const go = vi.fn();

function Harness({ reg, cmds }: { reg: ShortcutRegistry; cmds: CommandRegistry }) {
  useActions(
    buildGlobalActions({ ws: "acme", teams: [], currentTeamKey: null, go, logout: () => {} }),
    { shortcuts: reg, commands: cmds },
  );
  return null;
}

function press(init: KeyboardEventInit, target: Element | Document = document) {
  const e = new KeyboardEvent("keydown", {
    key: "f",
    code: "KeyF",
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}

function searchInput() {
  const input = document.createElement("input");
  input.setAttribute("data-search-target", "");
  return input;
}

describe("Cmd/Ctrl+F", () => {
  let reg: ShortcutRegistry;
  let cmds: CommandRegistry;
  let detach: () => void;
  beforeEach(() => {
    go.mockClear();
    reg = new ShortcutRegistry();
    cmds = new CommandRegistry();
    detach = attachShortcuts(document, reg);
    document.body.innerHTML = "";
  });
  afterEach(() => detach());

  it("focuses the page search input (Cmd and Ctrl) and prevents the browser find", () => {
    const input = searchInput();
    document.body.append(input);
    render(<Harness reg={reg} cmds={cmds} />);
    for (const init of [{ metaKey: true }, { ctrlKey: true }]) {
      input.blur();
      const e = press(init);
      expect(e.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(input);
    }
    expect(go).not.toHaveBeenCalled();
  });

  it("never navigates to the search page, even without a search input", () => {
    render(<Harness reg={reg} cmds={cmds} />);
    const e = press({ ctrlKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(go).not.toHaveBeenCalled();
  });

  it("does not intercept a second press while the input is focused", () => {
    const input = searchInput();
    document.body.append(input);
    render(<Harness reg={reg} cmds={cmds} />);
    press({ metaKey: true });
    const e = press({ metaKey: true }, input);
    expect(e.defaultPrevented).toBe(false);
    expect(go).not.toHaveBeenCalled();
  });

  it("works from within a textarea", () => {
    const input = searchInput();
    const ta = document.createElement("textarea");
    document.body.append(input, ta);
    render(<Harness reg={reg} cmds={cmds} />);
    ta.focus();
    const e = press({ ctrlKey: true }, ta);
    expect(e.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(input);
  });

  it("is listed with its shortcut for the help", () => {
    render(<Harness reg={reg} cmds={cmds} />);
    expect(cmds.list().find((c) => c.id === "search.focus")?.shortcut).toBe("mod+f");
  });
});
