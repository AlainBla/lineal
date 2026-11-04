import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { mockGraphql, renderWithApp } from "@/features/issues/test-fixtures";

import { BotsSection } from "./bots-section";

afterEach(() => vi.unstubAllGlobals());

const KEY = {
  id: "key-1",
  label: "CI",
  keyPrefix: "lin_api_abcd",
  scopes: ["read", "write"],
  teams: null,
  createdAt: "2026-11-01T00:00:00Z",
  lastUsedAt: null,
  expiresAt: null,
  revokedAt: null,
};

const BOT = {
  id: "bot-1",
  name: "Lineal Bot",
  displayName: "lineal-bot",
  email: "lineal-bot@acme.example",
  active: true,
  createdAt: "2026-11-01T00:00:00Z",
  apiKeys: [KEY],
};

describe("Bots (LIN-22)", () => {
  it("legt einen Bot an", async () => {
    const gql = mockGraphql({
      SettingsBots: () => ({ bots: [] }),
      SettingsBotCreate: () => ({ botCreate: { success: true, bot: { ...BOT, apiKeys: [] } } }),
    });
    await renderWithApp(<BotsSection />);
    expect(await screen.findByText("No bots yet.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "New bot" }));
    const dialog = await screen.findByRole("dialog", { name: "New bot" });
    await userEvent.type(within(dialog).getByLabelText("Bot name"), "Lineal Bot");
    await userEvent.type(within(dialog).getByLabelText("Bot email"), "lineal-bot@acme.example");
    await userEvent.click(within(dialog).getByRole("button", { name: "Create bot" }));
    await waitFor(() => expect(gql.of("SettingsBotCreate")).toHaveLength(1));
    expect(gql.of("SettingsBotCreate")[0]!.variables).toEqual({
      input: { name: "Lineal Bot", email: "lineal-bot@acme.example" },
    });
  });

  it("erzeugt einen Key für den Bot und zeigt den Token einmal", async () => {
    const gql = mockGraphql({
      SettingsBots: () => ({ bots: [BOT] }),
      SettingsBotApiKeyCreate: () => ({
        botApiKeyCreate: { success: true, token: "lin_api_secret", apiKey: KEY },
      }),
    });
    await renderWithApp(<BotsSection />);
    const row = await screen.findByRole("listitem", { name: "Bot Lineal Bot" });
    expect(within(row).getByText(/@lineal-bot/)).toBeInTheDocument();
    expect(within(row).getByRole("listitem", { name: "API key CI" })).toBeInTheDocument();

    await userEvent.click(within(row).getByRole("button", { name: "New key for Lineal Bot" }));
    const dialog = await screen.findByRole("dialog", { name: "New API key for Lineal Bot" });
    await userEvent.type(within(dialog).getByLabelText("Label"), "Agent");
    await userEvent.click(within(dialog).getByLabelText("Write"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Create key" }));
    await waitFor(() => expect(gql.of("SettingsBotApiKeyCreate")).toHaveLength(1));
    expect(gql.of("SettingsBotApiKeyCreate")[0]!.variables).toEqual({
      botId: "bot-1",
      input: { label: "Agent", scopes: ["READ", "WRITE"] },
    });
    expect(await screen.findByLabelText("API token")).toHaveValue("lin_api_secret");
  });

  it("widerruft einen Key des Bots", async () => {
    const gql = mockGraphql({
      SettingsBots: () => ({ bots: [BOT] }),
      SettingsBotApiKeyRevoke: () => ({ botApiKeyRevoke: { success: true } }),
    });
    await renderWithApp(<BotsSection />);
    const row = await screen.findByRole("listitem", { name: "Bot Lineal Bot" });
    await userEvent.click(within(row).getByRole("button", { name: "Revoke CI" }));
    const dialog = await screen.findByRole("dialog", { name: "Revoke API key?" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Revoke key" }));
    await waitFor(() => expect(gql.of("SettingsBotApiKeyRevoke")).toHaveLength(1));
    expect(gql.of("SettingsBotApiKeyRevoke")[0]!.variables).toEqual({
      botId: "bot-1",
      id: "key-1",
    });
  });
});
