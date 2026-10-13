import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearAssetCache, isAssetUrl, resolveAssetUrl, sameOrigin, uploadFile } from "./assets";
import { mockGraphql } from "./test-fixtures";

const ASSET = "http://localhost:8080/assets/org-1/aaaa-1111/bbbb-2222";

describe("Asset-URLs (F.08)", () => {
  beforeEach(() => clearAssetCache());
  afterEach(() => vi.unstubAllGlobals());

  it("erkennt dauerhafte Asset-URLs", () => {
    expect(isAssetUrl(ASSET)).toBe(true);
    expect(isAssetUrl("https://example.com/a.png")).toBe(false);
    expect(isAssetUrl(null)).toBe(false);
  });

  it("reduziert signierte URLs auf den eigenen Ursprung", () => {
    expect(sameOrigin("http://localhost:8080/files/tok?x=1")).toBe("/files/tok?x=1");
  });

  it("löst mehrere URLs in einem Aufruf auf und cached das Ergebnis", async () => {
    const other = `${ASSET}x`;
    const gql = mockGraphql({
      ResolveAssets: (v) => {
        const urls = v.urls as string[];
        return {
          resolveAssets: urls
            .filter((u) => u === ASSET)
            .map((u) => ({
              assetUrl: u,
              downloadUrl: "http://localhost:8080/files/signed",
              expiresAt: new Date(Date.now() + 300_000).toISOString(),
            })),
        };
      },
    });
    const [a, b] = await Promise.all([resolveAssetUrl(ASSET), resolveAssetUrl(other)]);
    expect(a).toBe("/files/signed");
    expect(b).toBeNull();
    expect(gql.of("ResolveAssets")).toHaveLength(1);
    expect(gql.of("ResolveAssets")[0]!.variables.urls).toEqual([ASSET, other]);
    expect(await resolveAssetUrl(ASSET)).toBe("/files/signed");
    expect(gql.of("ResolveAssets")).toHaveLength(1);
  });

  it("lädt per fileUpload und PUT hoch, ohne Content-Length zu setzen", async () => {
    const gql = mockGraphql({
      FileUpload: () => ({
        fileUpload: {
          success: true,
          uploadFile: {
            uploadUrl: "http://localhost:8080/uploads/tok",
            assetUrl: ASSET,
            headers: [
              { key: "content-type", value: "image/png" },
              { key: "content-length", value: "3" },
            ],
          },
        },
      }),
    });
    const base = globalThis.fetch as unknown as (...a: unknown[]) => Promise<Response>;
    const put = vi.fn<(url: unknown, init: unknown) => Promise<Response>>(
      async () => new Response(null, { status: 200 }),
    );
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) =>
      init?.method === "PUT" ? put(input, init) : base(input, init),
    );
    const file = new File([new Uint8Array([1, 2, 3])], "bild.png", { type: "image/png" });
    const res = await uploadFile(file);
    expect(res).toEqual({ assetUrl: ASSET, filename: "bild.png" });
    expect(gql.of("FileUpload")[0]!.variables).toEqual({
      filename: "bild.png",
      contentType: "image/png",
      size: 3,
    });
    const call = put.mock.calls[0] as unknown as [string, RequestInit];
    expect(call[0]).toBe("/uploads/tok");
    const headers = call[1].headers as Headers;
    expect(headers.get("content-type")).toBe("image/png");
    expect(headers.has("content-length")).toBe(false);
  });
});
