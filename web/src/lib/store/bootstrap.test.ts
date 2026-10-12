import { describe, expect, it, vi } from "vitest";

import { fetchBootstrap, parseBootstrapPage } from "./bootstrap";

const meta = (m: Record<string, unknown>) => `_metadata_=${JSON.stringify(m)}`;
const page = (lines: unknown[], m: Record<string, unknown>) =>
  [...lines.map((l) => JSON.stringify(l)), meta(m)].join("\n") + "\n";

describe("parseBootstrapPage", () => {
  it("trennt Objekte und Metadaten", () => {
    const out = parseBootstrapPage(
      page([{ __class: "User", id: "u1", name: "Alice" }], { lastSyncId: 7, nextCursor: null }),
    )!;
    expect(out.objects).toHaveLength(1);
    expect(out.meta.lastSyncId).toBe(7);
  });
  it("lehnt fehlende Metadaten und kaputte Zeilen ab", () => {
    expect(parseBootstrapPage(JSON.stringify({ __class: "User", id: "u" }) + "\n")).toBeNull();
    expect(parseBootstrapPage("{nope\n" + meta({ lastSyncId: 1 }))).toBeNull();
    expect(parseBootstrapPage(meta({ lastSyncId: -1 }))).toBeNull();
  });
});

describe("fetchBootstrap", () => {
  it("folgt dem Cursor und behält die lastSyncId des ersten Pakets", async () => {
    const calls: string[] = [];
    const fetchImpl = vi.fn(async (url: string) => {
      calls.push(url);
      const first = !url.includes("cursor=");
      return new Response(
        first
          ? page([{ __class: "User", id: "a" }], { lastSyncId: 10, nextCursor: "10;0;x;User" })
          : page([{ __class: "User", id: "b" }], { lastSyncId: 99, nextCursor: null }),
        { status: 200 },
      );
    });
    const res = await fetchBootstrap(["User"], { fetchImpl: fetchImpl as unknown as typeof fetch });
    expect(res.lastSyncId).toBe(10);
    expect(res.objects.map((o) => o.id)).toEqual(["a", "b"]);
    expect(calls[0]).toContain("modelNames=User");
    expect(calls[1]).toContain("cursor=");
    expect(calls[1]).not.toContain("modelNames");
  });
  it("wirft bei HTTP-Fehlern", async () => {
    const fetchImpl = vi.fn(async () => new Response("no", { status: 401 }));
    await expect(
      fetchBootstrap(["User"], { fetchImpl: fetchImpl as unknown as typeof fetch }),
    ).rejects.toThrow(/401/);
  });
});
