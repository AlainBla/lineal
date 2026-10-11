import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { acceptInvite, type AcceptResult } from "./accept-invite";
import { InvitePage, validateAccept } from "./invite-page";

const jsonRes = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("acceptInvite", () => {
  it("posts token, email, name and password with the CSRF header", async () => {
    const fetchImpl = vi.fn(async () => jsonRes(200, { workspace: "acme" }));
    const r = await acceptInvite(
      {
        token: "tok",
        email: " dora@acme.example ",
        name: " Dora ",
        password: "ein-gutes-passwort",
      },
      fetchImpl as unknown as typeof fetch,
    );
    expect(r).toEqual({ ok: true, workspace: "acme" });
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/invite/accept");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["x-lineal-csrf"]).toBe("1");
    expect(JSON.parse(String(init.body))).toEqual({
      token: "tok",
      email: "dora@acme.example",
      name: "Dora",
      password: "ein-gutes-passwort",
    });
  });

  it("ignores an unsafe workspace value from the server", async () => {
    const fetchImpl = vi.fn(async () => jsonRes(200, { workspace: "//evil.example" }));
    const r = await acceptInvite(
      { token: "t", email: "a@b.cc", name: "A", password: "12345678" },
      fetchImpl as unknown as typeof fetch,
    );
    expect(r).toEqual({ ok: true, workspace: null });
  });

  it.each([
    [404, "invalid_link"],
    [409, "account_exists"],
    [429, "rate_limited"],
    [403, "rejected"],
    [500, "server_error"],
  ])("maps status %i to %s", async (status, reason) => {
    const fetchImpl = vi.fn(async () => jsonRes(status, { error: "x" }));
    const r = await acceptInvite(
      { token: "t", email: "a@b.cc", name: "A", password: "12345678" },
      fetchImpl as unknown as typeof fetch,
    );
    expect(r).toMatchObject({ ok: false, reason });
  });

  it("reports the offending field on 400 and network errors", async () => {
    const bad = vi.fn(async () => jsonRes(400, { details: { field: "password" } }));
    expect(
      await acceptInvite(
        { token: "t", email: "a@b.cc", name: "A", password: "1" },
        bad as unknown as typeof fetch,
      ),
    ).toEqual({ ok: false, reason: "bad_input", field: "password" });
    const down = vi.fn(async () => {
      throw new TypeError("fail");
    });
    expect(
      await acceptInvite(
        { token: "t", email: "a@b.cc", name: "A", password: "12345678" },
        down as unknown as typeof fetch,
      ),
    ).toEqual({ ok: false, reason: "network" });
  });
});

describe("validateAccept", () => {
  it("requires email, name, a password of 8+ characters and a matching repeat", () => {
    expect(validateAccept({ email: "x", name: "", password: "kurz", confirm: "anders" })).toEqual({
      email: expect.any(String),
      name: expect.any(String),
      password: expect.any(String),
      confirm: expect.any(String),
    });
    expect(
      validateAccept({
        email: "dora@acme.example",
        name: "Dora",
        password: "12345678",
        confirm: "12345678",
      }),
    ).toEqual({});
  });
});

async function fillForm(user: ReturnType<typeof userEvent.setup>, password = "ein-gutes-passwort") {
  await user.type(screen.getByLabelText("Email"), "dora@acme.example");
  await user.type(screen.getByLabelText("Name"), "Dora Neu");
  await user.type(screen.getByLabelText("Password"), password);
  await user.type(screen.getByLabelText("Repeat password"), password);
}

describe("InvitePage", () => {
  it("submits the form and reports the workspace", async () => {
    const submit = vi.fn(async (): Promise<AcceptResult> => ({ ok: true, workspace: "acme" }));
    const onAccepted = vi.fn();
    await renderWithApp(
      <InvitePage token="tok123" session={null} onAccepted={onAccepted} submit={submit} />,
      newClient(),
      "/invite/tok123",
    );
    const user = userEvent.setup();
    await fillForm(user);
    await user.click(screen.getByRole("button", { name: /Create account and join/ }));
    await waitFor(() => expect(onAccepted).toHaveBeenCalledWith("acme"));
    expect(submit).toHaveBeenCalledWith({
      token: "tok123",
      email: "dora@acme.example",
      name: "Dora Neu",
      password: "ein-gutes-passwort",
    });
  });

  it("does not submit with a short password or mismatching repeat", async () => {
    const submit = vi.fn();
    await renderWithApp(
      <InvitePage token="t" session={null} onAccepted={vi.fn()} submit={submit} />,
      newClient(),
      "/invite/t",
    );
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Email"), "dora@acme.example");
    await user.type(screen.getByLabelText("Name"), "Dora");
    await user.type(screen.getByLabelText("Password"), "kurz");
    await user.type(screen.getByLabelText("Repeat password"), "anders");
    await user.click(screen.getByRole("button", { name: /Create account and join/ }));
    expect(await screen.findByText(/at least 8 characters/)).toBeInTheDocument();
    expect(screen.getByText(/do not match/)).toBeInTheDocument();
    expect(submit).not.toHaveBeenCalled();
  });

  it("shows one neutral message for invalid, expired, revoked or used links", async () => {
    const submit = vi.fn(async (): Promise<AcceptResult> => ({
      ok: false,
      reason: "invalid_link",
    }));
    await renderWithApp(
      <InvitePage token="t" session={null} onAccepted={vi.fn()} submit={submit} />,
      newClient(),
      "/invite/t",
    );
    const user = userEvent.setup();
    await fillForm(user);
    await user.click(screen.getByRole("button", { name: /Create account and join/ }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/no longer valid/);
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
  });

  it("offers to accept with the existing account when signed in", async () => {
    const submitAccount = vi.fn(async (): Promise<AcceptResult> => ({
      ok: true,
      workspace: "acme",
    }));
    const onAccepted = vi.fn();
    await renderWithApp(
      <InvitePage
        token="tok"
        session={SESSION}
        onAccepted={onAccepted}
        submitAccount={submitAccount}
      />,
      newClient(),
      "/invite/tok",
    );
    expect(screen.getByText("bob@acme.example")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Accept with this account" }));
    await waitFor(() => expect(onAccepted).toHaveBeenCalledWith("acme"));
    expect(submitAccount).toHaveBeenCalledWith("tok");
  });

  it("lets a signed-in user switch to the new-account form", async () => {
    await renderWithApp(
      <InvitePage token="tok" session={SESSION} onAccepted={vi.fn()} />,
      newClient(),
      "/invite/tok",
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Create a new account instead/ }));
    expect(screen.getByLabelText("Repeat password")).toBeInTheDocument();
  });
});
