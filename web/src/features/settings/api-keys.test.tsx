import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { mockGraphql, newClient, renderWithApp, SESSION } from "@/features/issues/test-fixtures";

import { ApiSettingsPage, endOfDayIso } from "./api-keys";

afterEach(() => vi.unstubAllGlobals());

const KEY = {
  id: "key-1",
  label: "CI pipeline",
  keyPrefix: "lin_api_ab12",
  scopes: ["read", "write"],
  teams: [{ id: "team-eng", key: "ENG", name: "Engineering" }],
  createdAt: "2026-10-01T10:00:00Z",
  lastUsedAt: null,
  expiresAt: null,
  revokedAt: null,
};
const REVOKED = {
  ...KEY,
  id: "key-2",
  label: "Old script",
  teams: null,
  revokedAt: "2026-10-02T10:00:00Z",
};

const render = () =>
  renderWithApp(<ApiSettingsPage ws="acme" />, newClient(), "/acme/settings/api");

describe("ApiSettingsPage", () => {
  it("lists keys without secrets and offers revoke only for active keys", async () => {
    mockGraphql({ SettingsApiKeys: () => ({ apiKeys: [KEY, REVOKED] }) });
    await render();
    const active = await screen.findByRole("listitem", { name: "API key CI pipeline" });
    expect(within(active).getByText("lin_api_ab12…")).toBeInTheDocument();
    expect(within(active).getByText(/read, write · ENG/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Revoke CI pipeline" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke Old script" })).not.toBeInTheDocument();
    expect(screen.getByText("Revoked")).toBeInTheDocument();
  });

  it("guests cannot create keys (B5.36)", async () => {
    mockGraphql({ SettingsApiKeys: () => ({ apiKeys: [] }) });
    const client = newClient();
    client.setQueryData(SESSION_QUERY_KEY, {
      ...SESSION,
      user: { ...SESSION.user, role: "guest" },
    });
    await renderWithApp(<ApiSettingsPage ws="acme" />, client, "/acme/settings/api");
    expect(await screen.findByText("Guests cannot create API keys.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New key" })).not.toBeInTheDocument();
  });

  it("shows an empty state", async () => {
    mockGraphql({ SettingsApiKeys: () => ({ apiKeys: [] }) });
    await render();
    expect(await screen.findByText("You have no API keys yet.")).toBeInTheDocument();
  });

  it("creates a key and shows the token once with a warning", async () => {
    const gql = mockGraphql({
      SettingsApiKeys: () => ({ apiKeys: [] }),
      SettingsApiKeyCreate: () => ({
        apiKeyCreate: { success: true, token: "lin_api_SECRET123", apiKey: KEY },
      }),
    });
    await render();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New key" }));
    await user.type(screen.getByLabelText("Label"), "  CI pipeline ");
    await user.click(screen.getByLabelText("Write"));
    await user.click(screen.getByLabelText("Team Engineering"));
    await user.click(screen.getByRole("button", { name: "Create key" }));
    await waitFor(() => expect(gql.of("SettingsApiKeyCreate")).toHaveLength(1));
    expect(gql.of("SettingsApiKeyCreate")[0]!.variables).toEqual({
      input: { label: "CI pipeline", scopes: ["READ", "WRITE"], teamIds: ["team-eng"] },
    });
    expect(await screen.findByLabelText("API token")).toHaveValue("lin_api_SECRET123");
    expect(screen.getByRole("alert")).toHaveTextContent(/only once/);
    await user.click(screen.getByRole("button", { name: "Done" }));
    // Token ist nach dem Schließen nirgends mehr im DOM.
    await waitFor(() => expect(screen.queryByLabelText("API token")).not.toBeInTheDocument());
    expect(document.body.textContent).not.toContain("lin_api_SECRET123");
  });

  it("defaults to read-only for all teams and validates the label", async () => {
    const gql = mockGraphql({
      SettingsApiKeys: () => ({ apiKeys: [] }),
      SettingsApiKeyCreate: () => ({
        apiKeyCreate: { success: true, token: "lin_api_x", apiKey: KEY },
      }),
    });
    await render();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New key" }));
    expect(screen.getByRole("button", { name: "Create key" })).toBeDisabled();
    await user.type(screen.getByLabelText("Label"), "x");
    await user.type(screen.getByLabelText("Expires"), "2099-01-31");
    await user.click(screen.getByRole("button", { name: "Create key" }));
    await waitFor(() => expect(gql.of("SettingsApiKeyCreate")).toHaveLength(1));
    const { input } = gql.of("SettingsApiKeyCreate")[0]!.variables as {
      input: Record<string, unknown>;
    };
    expect(input.scopes).toEqual(["READ"]);
    expect(input).not.toHaveProperty("teamIds");
    expect(input.expiresAt).toBe(endOfDayIso("2099-01-31"));
  });

  it("shows server errors in the dialog", async () => {
    mockGraphql({
      SettingsApiKeys: () => ({ apiKeys: [] }),
      SettingsApiKeyCreate: () => new Error("at most 25 active API keys per user"),
    });
    await render();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New key" }));
    await user.type(screen.getByLabelText("Label"), "x");
    await user.click(screen.getByRole("button", { name: "Create key" }));
    expect(await screen.findByText(/at most 25 active/)).toBeInTheDocument();
  });

  it("revokes only after confirmation", async () => {
    const gql = mockGraphql({
      SettingsApiKeys: () => ({ apiKeys: [KEY] }),
      SettingsApiKeyRevoke: () => ({ apiKeyRevoke: { success: true } }),
    });
    await render();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Revoke CI pipeline" }));
    expect(gql.of("SettingsApiKeyRevoke")).toHaveLength(0);
    await user.click(await screen.findByRole("button", { name: "Revoke key" }));
    await waitFor(() => expect(gql.of("SettingsApiKeyRevoke")).toHaveLength(1));
    expect(gql.of("SettingsApiKeyRevoke")[0]!.variables).toEqual({ id: "key-1" });
  });

  it("marks the API entry as current in the settings navigation", async () => {
    mockGraphql({ SettingsApiKeys: () => ({ apiKeys: [] }) });
    await render();
    expect(await screen.findByRole("link", { name: "API" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });
});
