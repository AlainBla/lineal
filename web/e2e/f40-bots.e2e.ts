import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";
import { callTool } from "./mcp";

// LIN-22: Admin legt einen Bot an, erzeugt einen Key; der Key handelt als Bot.
test("admin creates a bot whose API key comments as the bot", async ({ page, request }) => {
  await login(page, "alice");
  const id = await createIssue(page, "ENG", unique("Für den Bot"));

  await page.goto("/acme/settings/members");
  await page.getByRole("button", { name: "New bot" }).click();
  const create = page.getByRole("dialog", { name: "New bot" });
  const suffix = Math.random().toString(36).slice(2, 7);
  const name = `E2E Bot ${suffix}`;
  await create.getByLabel("Bot name").fill(name);
  await create.getByLabel("Bot email").fill(`e2e-bot-${suffix}@acme.example`);
  await create.getByRole("button", { name: "Create bot" }).click();
  await expect(create).toBeHidden();
  const row = page.getByRole("listitem", { name: `Bot ${name}` });
  await expect(row).toContainText(`@e2e-bot-${suffix}`);

  await row.getByRole("button", { name: `New key for ${name}` }).click();
  const dialog = page.getByRole("dialog", { name: `New API key for ${name}` });
  await dialog.getByLabel("Label").fill("agent");
  await dialog.getByLabel("Write").check();
  await dialog.getByRole("button", { name: "Create key" }).click();
  const token = await page.getByLabel("API token").inputValue();
  expect(token).toMatch(/^lin_api_/);
  await page.getByRole("button", { name: "Done" }).click();
  await expect(row.getByRole("listitem", { name: "API key agent" })).toBeVisible();

  const body = unique("Hallo vom Bot");
  const res = await callTool(request, token, "save_comment", { issueId: id, body });
  expect(res.isError).toBe(false);

  await page.goto(`/acme/issue/${id}`);
  // Der Kommentar stammt vom Bot (Anzeigename aus der E-Mail).
  await expect(page.getByRole("article", { name: `Comment by e2e-bot-${suffix}` })).toContainText(
    body,
  );
});
