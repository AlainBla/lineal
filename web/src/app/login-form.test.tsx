import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { login, normalizeSession, validateLogin } from "@/app/auth";
import { LoginForm } from "@/app/login-form";

describe("validateLogin", () => {
  it("requires email and password", () => {
    expect(validateLogin({ email: "", password: "" })).toEqual({
      email: "Enter your email address.",
      password: "Enter your password.",
    });
  });
  it("rejects malformed emails and accepts valid input", () => {
    expect(validateLogin({ email: "alice", password: "x" }).email).toMatch(/valid email/);
    expect(validateLogin({ email: "a@b", password: "x" }).email).toMatch(/valid email/);
    expect(validateLogin({ email: " alice@acme.example ", password: "x" })).toEqual({});
  });
});

describe("LoginForm", () => {
  it("shows field errors, focuses the first invalid field and does not submit", async () => {
    const user = userEvent.setup();
    const submit = vi.fn();
    render(<LoginForm onSuccess={() => {}} submit={submit} />);
    await user.click(screen.getByRole("button", { name: "Log in" }));
    const email = screen.getByLabelText("Email");
    expect(email).toHaveAttribute("aria-invalid", "true");
    expect(email).toHaveFocus();
    expect(screen.getByText("Enter your email address.")).toBeInTheDocument();
    expect(screen.getByText("Enter your password.")).toBeInTheDocument();
    expect(submit).not.toHaveBeenCalled();
  });

  it("submits trimmed credentials and calls onSuccess", async () => {
    const user = userEvent.setup();
    const submit = vi.fn().mockResolvedValue({ ok: true });
    const onSuccess = vi.fn();
    render(<LoginForm onSuccess={onSuccess} submit={submit} />);
    await user.type(screen.getByLabelText("Email"), " alice@acme.example ");
    await user.type(screen.getByLabelText("Password"), "s3cret");
    await user.click(screen.getByRole("button", { name: "Log in" }));
    await waitFor(() => expect(onSuccess).toHaveBeenCalledTimes(1));
    expect(submit).toHaveBeenCalledWith("alice@acme.example", "s3cret");
  });

  it("shows a server error for wrong credentials", async () => {
    const user = userEvent.setup();
    const submit = vi.fn().mockResolvedValue({ ok: false, reason: "invalid_credentials" });
    const onSuccess = vi.fn();
    render(<LoginForm onSuccess={onSuccess} submit={submit} />);
    await user.type(screen.getByLabelText("Email"), "alice@acme.example");
    await user.type(screen.getByLabelText("Password"), "nope");
    await user.click(screen.getByRole("button", { name: "Log in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect email or password.");
    expect(onSuccess).not.toHaveBeenCalled();
  });
});

describe("auth client", () => {
  it("posts JSON credentials to /auth/login and maps status codes", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    expect(await login("a@b.co", "pw", fetchImpl)).toEqual({ ok: true });
    const [url, init] = fetchImpl.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/auth/login");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ email: "a@b.co", password: "pw" });
    expect(init.credentials).toBe("same-origin");

    const fail = (status: number) => vi.fn().mockResolvedValue(new Response("{}", { status }));
    expect(await login("a@b.co", "pw", fail(401))).toEqual({
      ok: false,
      reason: "invalid_credentials",
    });
    expect(await login("a@b.co", "pw", fail(429))).toEqual({ ok: false, reason: "rate_limited" });
    expect(await login("a@b.co", "pw", fail(500))).toEqual({ ok: false, reason: "server_error" });
    expect(await login("a@b.co", "pw", vi.fn().mockRejectedValue(new Error("x")))).toEqual({
      ok: false,
      reason: "network",
    });
  });

  it("normalizes the /auth/me payload", () => {
    expect(
      normalizeSession({
        user: { id: "u1", displayName: "alice", email: "alice@acme.example" },
        organization: { id: "o1", name: "Acme", urlKey: "acme" },
      }),
    ).toEqual({
      user: { id: "u1", name: "alice", email: "alice@acme.example", avatarUrl: null },
      workspace: { id: "o1", name: "Acme", urlKey: "acme" },
      teams: [],
    });
    expect(normalizeSession({})).toBeNull();
  });

  it("versteht das flache /auth/me-Format des Backends", () => {
    expect(
      normalizeSession({
        id: "u1",
        email: "bob@acme.example",
        displayName: "bob",
        workspace: { id: "w1", name: "Acme", urlKey: "acme" },
      }),
    ).toEqual({
      user: { id: "u1", email: "bob@acme.example", name: "bob", avatarUrl: null },
      workspace: { id: "w1", name: "Acme", urlKey: "acme" },
      teams: [],
    });
  });
});
