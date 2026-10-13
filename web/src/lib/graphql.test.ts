import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchSession } from "@/app/auth";
import { shouldRetry } from "@/app/query-client";

import {
  describeError,
  GraphQLRequestError,
  gqlRequest,
  NETWORK_ERROR_MESSAGE,
  parseRetryAfter,
  retryDelayMs,
  setUnauthenticatedHandler,
} from "./graphql";

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });

afterEach(() => setUnauthenticatedHandler(null));

describe("gqlRequest", () => {
  it("429: wartet Retry-After und wiederholt einmal (B4.07)", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(
        json(429, { errors: [{ message: "slow down" }] }, { "retry-after": "3" }),
      )
      .mockResolvedValueOnce(json(200, { data: { ok: true } }));
    const sleepImpl = vi.fn().mockResolvedValue(undefined);
    const data = await gqlRequest<{ ok: boolean }>("query { ok }", undefined, {
      fetchImpl: fetchImpl as unknown as typeof fetch,
      sleepImpl,
    });
    expect(data.ok).toBe(true);
    expect(sleepImpl).toHaveBeenCalledWith(3000);
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });

  it("429 zweimal: Fehler mit Retry-After und verständlichem Text", async () => {
    const fetchImpl = vi
      .fn()
      .mockImplementation(() =>
        Promise.resolve(json(429, { errors: [{ message: "x" }] }, { "retry-after": "7" })),
      );
    const err = await gqlRequest("query { ok }", undefined, {
      fetchImpl: fetchImpl as unknown as typeof fetch,
      sleepImpl: () => Promise.resolve(),
    }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(GraphQLRequestError);
    expect((err as GraphQLRequestError).retryAfterSeconds).toBe(7);
    expect(describeError(err)).toBe("Too many requests. Try again in 7 seconds.");
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });

  it("Netzwerkfehler: verständlicher Text statt „Failed to fetch“ (B4.24)", async () => {
    const fetchImpl = vi.fn().mockRejectedValue(new TypeError("Failed to fetch"));
    const err = await gqlRequest("query { ok }", undefined, {
      fetchImpl: fetchImpl as unknown as typeof fetch,
    }).catch((e: unknown) => e);
    expect((err as GraphQLRequestError).isNetwork).toBe(true);
    expect(describeError(err)).toBe(NETWORK_ERROR_MESSAGE);
    expect(describeError(new TypeError("Failed to fetch"))).toBe(NETWORK_ERROR_MESSAGE);
  });

  it("401 ruft den Session-Ablauf-Handler (B4.11)", async () => {
    const handler = vi.fn();
    setUnauthenticatedHandler(handler);
    const fetchImpl = vi
      .fn()
      .mockResolvedValue(
        json(401, { errors: [{ message: "no", extensions: { code: "unauthenticated" } }] }),
      );
    await expect(
      gqlRequest("query { ok }", undefined, { fetchImpl: fetchImpl as unknown as typeof fetch }),
    ).rejects.toBeInstanceOf(GraphQLRequestError);
    expect(handler).toHaveBeenCalledTimes(1);
  });
});

describe("Hilfsfunktionen", () => {
  it("Retry-After und Wartegrenze", () => {
    expect(parseRetryAfter("5")).toBe(5);
    expect(parseRetryAfter("Wed, 21 Oct 2026 07:28:00 GMT")).toBeUndefined();
    expect(retryDelayMs(undefined)).toBe(1000);
    expect(retryDelayMs(3600)).toBe(10_000);
  });

  it("Rechtefehler ohne rohe Aktionsnamen (B4.31)", () => {
    const err = new GraphQLRequestError("Not allowed to perform ViewWorkspaceOverview.", 200, [
      {
        message: "Not allowed to perform ViewWorkspaceOverview.",
        extensions: { code: "forbidden" },
      },
    ]);
    expect(describeError(err)).toBe("You do not have permission to do this.");
  });

  it("Query-Client wiederholt nur Netz- und 5xx-Fehler", () => {
    expect(shouldRetry(0, new GraphQLRequestError("x", 429))).toBe(false);
    expect(shouldRetry(0, new GraphQLRequestError("x", 401))).toBe(false);
    expect(shouldRetry(0, new GraphQLRequestError("x", 503))).toBe(true);
    expect(shouldRetry(0, new GraphQLRequestError("x", 0))).toBe(true);
    expect(shouldRetry(1, new GraphQLRequestError("x", 503))).toBe(false);
  });
});

describe("fetchSession bei 429 (B4.07)", () => {
  it("wartet Retry-After und wiederholt, dann Erfolg", async () => {
    const me = { id: "u1", email: "bob@acme.example", workspace: { urlKey: "acme", name: "Acme" } };
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response("", { status: 429, headers: { "retry-after": "1" } }))
      .mockResolvedValueOnce(json(200, me));
    const sleepImpl = vi.fn().mockResolvedValue(undefined);
    const s = await fetchSession(fetchImpl as unknown as typeof fetch, sleepImpl);
    expect(s?.workspace.urlKey).toBe("acme");
    expect(sleepImpl).toHaveBeenCalledWith(1000);
  });

  it("zweimal 429: freundliche Fehlermeldung statt „Session check failed (429)“", async () => {
    const fetchImpl = vi
      .fn()
      .mockImplementation(() => Promise.resolve(new Response("", { status: 429 })));
    await expect(
      fetchSession(fetchImpl as unknown as typeof fetch, () => Promise.resolve()),
    ).rejects.toThrow(/Too many requests/);
  });
});
