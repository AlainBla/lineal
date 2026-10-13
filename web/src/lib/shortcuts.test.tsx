import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  formatKeys,
  matchesCombo,
  parseCombo,
  parseSequence,
  SEQUENCE_TIMEOUT_MS,
  ShortcutRegistry,
  type KeyLike,
} from "./shortcuts";

function key(k: string, mods: Partial<KeyLike> = {}): KeyLike {
  return { key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods };
}

describe("parsing", () => {
  it("parses combos and sequences", () => {
    expect(parseCombo("mod+shift+1")).toMatchObject({ key: "1", mod: true, shift: true });
    expect(parseCombo("esc").key).toBe("escape");
    expect(parseSequence("g i")).toHaveLength(2);
    expect(() => parseCombo("hyper+x")).toThrow();
  });

  it("matches mod as ctrl or meta, and ignores shift for symbols", () => {
    const k = parseCombo("mod+k");
    expect(matchesCombo(key("k", { ctrlKey: true }), k)).toBe(true);
    expect(matchesCombo(key("k", { metaKey: true }), k)).toBe(true);
    expect(matchesCombo(key("k"), k)).toBe(false);
    expect(matchesCombo(key("?", { shiftKey: true }), parseCombo("?"))).toBe(true);
    expect(matchesCombo(key("k", { ctrlKey: true, shiftKey: true }), k)).toBe(false);
  });

  it("uses the physical key for punctuation with Ctrl/Alt (Ctrl ⇧ , und Ctrl ⇧ ')", () => {
    const comma = parseCombo("mod+shift+,");
    expect(
      matchesCombo({ ...key("<", { ctrlKey: true, shiftKey: true }), code: "Comma" }, comma),
    ).toBe(true);
    const quote = parseCombo("mod+shift+'");
    expect(
      matchesCombo({ ...key('"', { ctrlKey: true, shiftKey: true }), code: "Quote" }, quote),
    ).toBe(true);
    expect(
      matchesCombo({ ...key(".", { ctrlKey: true }), code: "Period" }, parseCombo("mod+.")),
    ).toBe(true);
    // Shift allein bleibt beim Zeichen: `?` und `#` hängen nicht von der Taste ab.
    expect(matchesCombo({ ...key("?", { shiftKey: true }), code: "Slash" }, parseCombo("?"))).toBe(
      true,
    );
  });

  it("uses the physical key for shifted digits", () => {
    const c = parseCombo("mod+shift+1");
    expect(
      matchesCombo({ ...key("!", { ctrlKey: true, shiftKey: true }), code: "Digit1" }, c),
    ).toBe(true);
  });

  it("formats keys for display", () => {
    expect(formatKeys("mod+k", false)).toEqual([["Ctrl", "K"]]);
    expect(formatKeys("mod+k", true)).toEqual([["⌘", "K"]]);
    expect(formatKeys("g i")).toEqual([["G"], ["I"]]);
  });
});

describe("ShortcutRegistry", () => {
  let reg: ShortcutRegistry;
  beforeEach(() => {
    vi.useFakeTimers();
    reg = new ShortcutRegistry();
  });
  afterEach(() => {
    reg.reset();
    vi.useRealTimers();
  });

  it("runs a single-key shortcut and reports it as handled", () => {
    const run = vi.fn();
    reg.register({ id: "a", keys: "c", description: "Create", run });
    expect(reg.handleKeyDown(key("c"))).toBe(true);
    expect(run).toHaveBeenCalledTimes(1);
    expect(reg.handleKeyDown(key("x"))).toBe(false);
  });

  it("supports sequences like `g i` and `g m`", () => {
    const inbox = vi.fn();
    const mine = vi.fn();
    reg.register({ id: "inbox", keys: "g i", description: "Inbox", run: inbox });
    reg.register({ id: "mine", keys: "g m", description: "My issues", run: mine });

    expect(reg.handleKeyDown(key("g"))).toBe(true); // Präfix wird verbraucht
    expect(inbox).not.toHaveBeenCalled();
    expect(reg.handleKeyDown(key("i"))).toBe(true);
    expect(inbox).toHaveBeenCalledTimes(1);

    reg.handleKeyDown(key("g"));
    reg.handleKeyDown(key("m"));
    expect(mine).toHaveBeenCalledTimes(1);
    expect(inbox).toHaveBeenCalledTimes(1);
  });

  it("drops a pending sequence after the timeout", () => {
    const run = vi.fn();
    reg.register({ id: "inbox", keys: "g i", description: "Inbox", run });
    reg.handleKeyDown(key("g"));
    vi.advanceTimersByTime(SEQUENCE_TIMEOUT_MS + 10);
    expect(reg.handleKeyDown(key("i"))).toBe(false);
    expect(run).not.toHaveBeenCalled();
  });

  it("aborts a sequence on a non-matching key and restarts", () => {
    const run = vi.fn();
    reg.register({ id: "inbox", keys: "g i", description: "Inbox", run });
    reg.handleKeyDown(key("g"));
    expect(reg.handleKeyDown(key("z"))).toBe(false);
    expect(reg.handleKeyDown(key("i"))).toBe(false);
    reg.handleKeyDown(key("g"));
    reg.handleKeyDown(key("i"));
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("verworfene Sequenz: die zweite Taste feuert kein Einzelkürzel (B4.10)", () => {
    const open = vi.fn();
    const priority = vi.fn();
    reg.register({ id: "o-c", keys: "o c", description: "Open", run: open });
    reg.register({ id: "p", keys: "p", description: "Priority", run: priority });
    reg.handleKeyDown(key("o"));
    expect(reg.handleKeyDown(key("p"))).toBe(true);
    expect(priority).not.toHaveBeenCalled();
    // Tasten ohne Kürzel (Tab, Enter) behalten nach abgebrochener Folge ihr Browserverhalten
    reg.handleKeyDown(key("o"));
    expect(reg.handleKeyDown(key("Tab"))).toBe(false);
    // danach funktioniert p wieder normal
    reg.handleKeyDown(key("p"));
    expect(priority).toHaveBeenCalledTimes(1);
  });

  it("supports shifted second steps (`g shift+w`)", () => {
    const run = vi.fn();
    reg.register({ id: "ws", keys: "g shift+w", description: "x", run });
    reg.handleKeyDown(key("g"));
    reg.handleKeyDown(key("W", { shiftKey: true }));
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("respects contexts", () => {
    const global = vi.fn();
    const list = vi.fn();
    const issue = vi.fn();
    reg.register({ id: "g", keys: "s", description: "global", run: global });
    reg.register({ id: "l", keys: "x", context: "list", description: "list", run: list });
    reg.register({ id: "i", keys: "x", context: "issue", description: "issue", run: issue });

    expect(reg.handleKeyDown(key("x"))).toBe(false); // kein Kontext aktiv
    const popList = reg.pushContext("list");
    reg.handleKeyDown(key("x"));
    expect(list).toHaveBeenCalledTimes(1);

    // spezifischerer Kontext gewinnt bei gleicher Taste
    const popIssue = reg.pushContext("issue");
    reg.handleKeyDown(key("x"));
    expect(issue).toHaveBeenCalledTimes(1);
    expect(list).toHaveBeenCalledTimes(1);

    popIssue();
    popList();
    expect(reg.activeContexts()).toEqual(["global"]);
    reg.handleKeyDown(key("s"));
    expect(global).toHaveBeenCalledTimes(1);
  });

  it("suppresses non-modal shortcuts while a modal is active", () => {
    const global = vi.fn();
    const modal = vi.fn();
    const palette = vi.fn();
    reg.register({ id: "g", keys: "c", description: "g", run: global });
    reg.register({ id: "m", keys: "enter", context: "modal", description: "m", run: modal });
    reg.register({ id: "p", keys: "mod+k", allowInModal: true, description: "p", run: palette });
    const pop = reg.pushContext("modal");
    reg.handleKeyDown(key("c"));
    reg.handleKeyDown(key("Enter"));
    reg.handleKeyDown(key("k", { ctrlKey: true }));
    expect(global).not.toHaveBeenCalled();
    expect(modal).toHaveBeenCalledTimes(1);
    expect(palette).toHaveBeenCalledTimes(1);
    pop();
    reg.handleKeyDown(key("c"));
    expect(global).toHaveBeenCalledTimes(1);
  });

  it("ignores plain keys typed into inputs but keeps modifier shortcuts", () => {
    const plain = vi.fn();
    const mod = vi.fn();
    reg.register({ id: "p", keys: "c", description: "p", run: plain });
    reg.register({ id: "m", keys: "mod+k", description: "m", run: mod });
    const input = document.createElement("input");
    reg.handleKeyDown({ ...key("c"), target: input });
    reg.handleKeyDown({ ...key("k", { ctrlKey: true }), target: input });
    expect(plain).not.toHaveBeenCalled();
    expect(mod).toHaveBeenCalledTimes(1);
  });

  it("unregisters and lets a handler decline the event", () => {
    const run = vi.fn();
    const off = reg.register({ id: "a", keys: "c", description: "c", run });
    off();
    expect(reg.handleKeyDown(key("c"))).toBe(false);
    reg.register({ id: "b", keys: "d", description: "d", run: () => false });
    expect(reg.handleKeyDown(key("d"))).toBe(false);
  });
});
