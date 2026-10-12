import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { rangeBetween, toggled, useSelection } from "./selection";

const IDS = ["a", "b", "c", "d", "e"];
const none = { shiftKey: false, metaKey: false, ctrlKey: false };

describe("rangeBetween", () => {
  it("liefert den Bereich in beide Richtungen", () => {
    expect(rangeBetween(IDS, "b", "d")).toEqual(["b", "c", "d"]);
    expect(rangeBetween(IDS, "d", "b")).toEqual(["b", "c", "d"]);
  });
  it("ohne Anker nur das Ziel, unbekanntes Ziel leer", () => {
    expect(rangeBetween(IDS, null, "c")).toEqual(["c"]);
    expect(rangeBetween(IDS, "zz", "c")).toEqual(["c"]);
    expect(rangeBetween(IDS, "a", "zz")).toEqual([]);
  });
});

describe("toggled", () => {
  it("schaltet um, ohne das Original zu verändern", () => {
    const base = new Set(["a"]);
    expect([...toggled(base, "b")]).toEqual(["a", "b"]);
    expect([...toggled(base, "a")]).toEqual([]);
    expect([...base]).toEqual(["a"]);
  });
});

describe("useSelection", () => {
  it("Cmd-Klick schaltet um, Shift-Klick wählt den Bereich, normaler Klick navigiert", () => {
    const { result } = renderHook(() => useSelection(IDS));
    expect(result.current.handleClick("a", none)).toBe(false);
    act(() => void result.current.handleClick("b", { ...none, metaKey: true }));
    expect([...result.current.selected]).toEqual(["b"]);
    act(() => void result.current.handleClick("d", { ...none, shiftKey: true }));
    expect([...result.current.selected].sort()).toEqual(["b", "c", "d"]);
    // Mit bestehender Auswahl wählt auch ein normaler Klick.
    act(() => void result.current.handleClick("e", none));
    expect(result.current.has("e")).toBe(true);
    act(() => result.current.clear());
    expect(result.current.size).toBe(0);
  });

  it("verwirft Issues, die nicht mehr angezeigt werden", () => {
    const { result, rerender } = renderHook(({ ids }) => useSelection(ids), {
      initialProps: { ids: IDS },
    });
    act(() => result.current.toggle("a"));
    act(() => result.current.toggle("b"));
    rerender({ ids: ["b", "c"] });
    expect([...result.current.selected]).toEqual(["b"]);
  });
});
