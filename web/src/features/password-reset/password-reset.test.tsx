import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { newClient, renderWithApp } from "@/features/issues/test-fixtures";

import { ForgotPasswordPage } from "./forgot-password-page";
import {
  requestPasswordReset,
  resetPassword,
  validateForgotEmail,
  validateNewPassword,
} from "./password-reset";
import { ResetPasswordPage } from "./reset-password-page";

const jsonRes = (status: number, body: unknown = {}) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("requestPasswordReset", () => {
  it("posts email (and urlKey when given) with the CSRF header", async () => {
    const fetchImpl = vi.fn(async () => jsonRes(200, { message: "ok" }));
    const r = await requestPasswordReset(
      " alice@acme.example ",
      "acme",
      fetchImpl as unknown as typeof fetch,
    );
    expect(r).toEqual({ kind: "sent" });
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/auth/forgot-password");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["x-lineal-csrf"]).toBe("1");
    expect(JSON.parse(String(init.body))).toEqual({ email: "alice@acme.example", urlKey: "acme" });
  });

  it("omits urlKey when empty and treats 429 as sent (no enumeration)", async () => {
    const fetchImpl = vi.fn(async () => jsonRes(429, {}));
    const r = await requestPasswordReset("a@b.cc", "", fetchImpl as unknown as typeof fetch);
    expect(r).toEqual({ kind: "sent" });
    expect(
      JSON.parse(String((fetchImpl.mock.calls[0] as unknown as [string, RequestInit])[1].body)),
    ).toEqual({
      email: "a@b.cc",
    });
  });

  it("maps 5xx to server_error and a thrown fetch to network", async () => {
    expect(
      await requestPasswordReset("a@b.cc", undefined, vi.fn(async () => jsonRes(500)) as never),
    ).toEqual({
      kind: "error",
      reason: "server_error",
    });
    expect(
      await requestPasswordReset(
        "a@b.cc",
        undefined,
        vi.fn(async () => {
          throw new TypeError("down");
        }) as never,
      ),
    ).toEqual({ kind: "error", reason: "network" });
  });
});

describe("resetPassword", () => {
  it("posts token and newPassword with the CSRF header", async () => {
    const fetchImpl = vi.fn(async () => jsonRes(200, { success: true }));
    const r = await resetPassword(
      "lin_reset_x",
      "fresh-password-123",
      fetchImpl as unknown as typeof fetch,
    );
    expect(r).toEqual({ kind: "ok" });
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/auth/reset-password");
    expect((init.headers as Record<string, string>)["x-lineal-csrf"]).toBe("1");
    expect(JSON.parse(String(init.body))).toEqual({
      token: "lin_reset_x",
      newPassword: "fresh-password-123",
    });
  });

  it("maps a weak-password 400 and an invalid-token 400 distinctly", async () => {
    expect(
      await resetPassword(
        "t",
        "short",
        vi.fn(async () => jsonRes(400, { details: { field: "password" } })) as never,
      ),
    ).toEqual({ kind: "error", reason: "weak_password" });
    expect(
      await resetPassword(
        "t",
        "fresh-password-123",
        vi.fn(async () => jsonRes(400, { details: { reason: "invalid_token" } })) as never,
      ),
    ).toEqual({ kind: "error", reason: "invalid_link" });
  });

  it("maps 429 and network errors", async () => {
    expect(
      await resetPassword("t", "fresh-password-123", vi.fn(async () => jsonRes(429)) as never),
    ).toEqual({
      kind: "error",
      reason: "rate_limited",
    });
    expect(
      await resetPassword(
        "t",
        "fresh-password-123",
        vi.fn(async () => {
          throw new TypeError("down");
        }) as never,
      ),
    ).toEqual({ kind: "error", reason: "network" });
  });
});

describe("validation", () => {
  it("validateForgotEmail rejects empty and malformed addresses", () => {
    expect(validateForgotEmail("")).toBeTruthy();
    expect(validateForgotEmail("nope")).toBeTruthy();
    expect(validateForgotEmail("alice@acme.example")).toBeUndefined();
  });

  it("validateNewPassword enforces min 12 and a matching repeat", () => {
    expect(validateNewPassword("short", "short")).toEqual({ password: expect.any(String) });
    expect(validateNewPassword("twelve-chars-ok", "different")).toEqual({
      confirm: expect.any(String),
    });
    expect(validateNewPassword("twelve-chars-ok", "twelve-chars-ok")).toEqual({});
  });
});

describe("ForgotPasswordPage", () => {
  it("shows the same generic confirmation after submitting", async () => {
    const submit = vi.fn(async () => ({ kind: "sent" as const }));
    await renderWithApp(<ForgotPasswordPage submit={submit} />, newClient(), "/forgot-password");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Email"), "alice@acme.example");
    await user.click(screen.getByRole("button", { name: /Send reset link/ }));
    await waitFor(() => expect(screen.getByText(/Check your email/)).toBeInTheDocument());
    expect(screen.getByText(/If an account exists/)).toBeInTheDocument();
    expect(submit).toHaveBeenCalledWith("alice@acme.example", undefined);
  });

  it("does not submit an invalid email", async () => {
    const submit = vi.fn(async () => ({ kind: "sent" as const }));
    await renderWithApp(<ForgotPasswordPage submit={submit} />, newClient(), "/forgot-password");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Email"), "not-an-email");
    await user.click(screen.getByRole("button", { name: /Send reset link/ }));
    expect(submit).not.toHaveBeenCalled();
  });
});

describe("ResetPasswordPage", () => {
  it("submits a new password and shows the confirmation", async () => {
    const submit = vi.fn(async () => ({ kind: "ok" as const }));
    const onDone = vi.fn();
    await renderWithApp(
      <ResetPasswordPage token="lin_reset_abc" onDone={onDone} submit={submit} />,
      newClient(),
      "/reset-password",
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("New password"), "fresh-password-123");
    await user.type(screen.getByLabelText("Repeat password"), "fresh-password-123");
    await user.click(screen.getByRole("button", { name: /Set new password/ }));
    await waitFor(() => expect(screen.getByText(/Password updated/)).toBeInTheDocument());
    expect(submit).toHaveBeenCalledWith("lin_reset_abc", "fresh-password-123");
    expect(onDone).toHaveBeenCalled();
  });

  it("shows an invalid-link message when the token is missing", async () => {
    await renderWithApp(
      <ResetPasswordPage token="" onDone={vi.fn()} submit={vi.fn()} />,
      newClient(),
      "/reset-password",
    );
    expect(screen.getByText(/Reset link not valid/)).toBeInTheDocument();
  });

  it("does not submit a short or mismatching password", async () => {
    const submit = vi.fn();
    await renderWithApp(
      <ResetPasswordPage token="lin_reset_abc" onDone={vi.fn()} submit={submit} />,
      newClient(),
      "/reset-password",
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("New password"), "short");
    await user.type(screen.getByLabelText("Repeat password"), "nope");
    await user.click(screen.getByRole("button", { name: /Set new password/ }));
    expect(submit).not.toHaveBeenCalled();
  });
});
