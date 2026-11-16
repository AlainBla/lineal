import { describe, expect, it, vi } from "vitest";

import { loginTotp, totpEnrollConfirm, totpStatus } from "@/app/totp-api";

describe("totp-api", () => {
  it("loginTotp maps status codes and sends the factor", async () => {
    const ok = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    expect(await loginTotp("pt", { code: "123456" }, ok)).toEqual({ ok: true });
    const [url, init] = ok.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/auth/login/totp");
    expect(JSON.parse(init.body as string)).toEqual({ pendingToken: "pt", code: "123456" });

    const status = (s: number) => vi.fn().mockResolvedValue(new Response("{}", { status: s }));
    expect(await loginTotp("pt", { code: "x" }, status(429))).toEqual({
      ok: false,
      reason: "rate_limited",
    });
    expect(await loginTotp("pt", { recoveryCode: "x" }, status(401))).toEqual({
      ok: false,
      reason: "invalid",
    });
  });

  it("totpStatus parses the payload", async () => {
    const f = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          enabled: true,
          confirmed: true,
          recoveryRemaining: 7,
          ownerEnforced: false,
          graceDaysLeft: null,
          graceLoginsLeft: null,
        }),
        { status: 200 },
      ),
    );
    const s = await totpStatus(f);
    expect(s.confirmed).toBe(true);
    expect(s.recoveryRemaining).toBe(7);
  });

  it("totpEnrollConfirm returns recovery codes and throws on error", async () => {
    const ok = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ recoveryCodes: ["A", "B"] }), { status: 200 }),
      );
    expect(await totpEnrollConfirm("123456", undefined, ok)).toEqual(["A", "B"]);

    const bad = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ message: "invalid code" }), { status: 400 }),
      );
    await expect(totpEnrollConfirm("000000", undefined, bad)).rejects.toThrow("invalid code");
  });
});
