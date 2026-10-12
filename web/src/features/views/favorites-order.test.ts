import { describe, expect, it } from "vitest";

import { between, computeFavoriteMove, type FavoriteItem } from "./favorites-order";

const f = (id: string, sortOrder: number, extra: Partial<FavoriteItem> = {}): FavoriteItem => ({
  id,
  type: "customView",
  sortOrder,
  parentId: null,
  ...extra,
});

describe("between", () => {
  it("Mittelwert, Rand und leere Ebene", () => {
    expect(between(1, 3)).toBe(2);
    expect(between(null, 2)).toBe(1);
    expect(between(2, null)).toBe(3);
    expect(between(null, null)).toBe(0);
  });
});

describe("computeFavoriteMove", () => {
  const items = [f("a", 1), f("b", 2), f("c", 3), f("dir", 4, { type: "folder" })];

  it("nach unten: hinter das Ziel", () => {
    expect(computeFavoriteMove(items, "a", "b")).toEqual({
      id: "a",
      parentId: null,
      sortOrder: 2.5,
    });
    expect(computeFavoriteMove(items, "a", "c")).toEqual({
      id: "a",
      parentId: null,
      sortOrder: 3.5,
    });
  });

  it("nach oben: vor das Ziel (auch an den Anfang)", () => {
    expect(computeFavoriteMove(items, "c", "b")).toEqual({
      id: "c",
      parentId: null,
      sortOrder: 1.5,
    });
    expect(computeFavoriteMove(items, "c", "a")).toEqual({
      id: "c",
      parentId: null,
      sortOrder: 0,
    });
  });

  it("auf sich selbst oder unbekannt: nichts", () => {
    expect(computeFavoriteMove(items, "a", "a")).toBeNull();
    expect(computeFavoriteMove(items, "a", "zzz")).toBeNull();
  });

  it("auf einen Ordner: ans Ende des Ordners", () => {
    const withKid = [...items, f("k", 10, { parentId: "dir" })];
    expect(computeFavoriteMove(withKid, "a", "dir")).toEqual({
      id: "a",
      parentId: "dir",
      sortOrder: 11,
    });
    expect(computeFavoriteMove(items, "b", "dir")).toEqual({
      id: "b",
      parentId: "dir",
      sortOrder: 0,
    });
  });

  it("Kind auf den eigenen Ordner: zurück auf die oberste Ebene hinter den Ordner", () => {
    const withKid = [...items, f("k", 10, { parentId: "dir" })];
    expect(computeFavoriteMove(withKid, "k", "dir")).toEqual({
      id: "k",
      parentId: null,
      sortOrder: 5,
    });
  });

  it("Umsortieren im Ordner", () => {
    const kids = [...items, f("k1", 1, { parentId: "dir" }), f("k2", 2, { parentId: "dir" })];
    expect(computeFavoriteMove(kids, "k1", "k2")).toEqual({
      id: "k1",
      parentId: "dir",
      sortOrder: 3,
    });
  });

  it("Ordner wandern nie in einen Ordner", () => {
    const kids = [...items, f("k1", 1, { parentId: "dir" })];
    expect(computeFavoriteMove(kids, "dir", "k1")).toBeNull();
    // Ordner auf Ordner: normales Umsortieren auf der obersten Ebene.
    const two = [...items, f("dir2", 0.5, { type: "folder" })];
    expect(computeFavoriteMove(two, "dir", "dir2")?.parentId).toBeNull();
  });

  it("Element aus dem Ordner auf ein oberstes Element: dort einsortieren", () => {
    const kids = [...items, f("k1", 1, { parentId: "dir" })];
    expect(computeFavoriteMove(kids, "k1", "b")).toEqual({
      id: "k1",
      parentId: null,
      sortOrder: 1.5,
    });
  });
});
