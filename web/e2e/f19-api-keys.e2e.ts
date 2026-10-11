import { expect, test } from "@playwright/test";

import { login, unique } from "./helpers";
import { API_BASE, callTool } from "./mcp";

// WP19: API-Key in der Oberfläche anlegen, per MCP benutzen, widerrufen.
test("API key created in the UI works against /mcp until it is revoked", async ({
  page,
  request,
}) => {
  await login(page, "bob");
  await page.goto("/acme/settings/api");
  await expect(page.getByRole("heading", { name: "API keys" })).toBeVisible();

  const label = unique("E2E key");
  await page.getByRole("button", { name: "New key" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Label").fill(label);
  await dialog.getByRole("button", { name: "Create key" }).click();

  // Der Token erscheint genau einmal; Hinweis sichtbar.
  await expect(dialog.getByRole("alert")).toBeVisible();
  const token = await dialog.getByLabel("API token").inputValue();
  expect(token).toMatch(/^lin_api_/);
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("listitem", { name: `API key ${label}` })).toBeVisible();
  await expect(page.locator("body")).not.toContainText(token);

  // Der Key funktioniert (Standard: read, alle Teams).
  const listed = await callTool(request, token, "list_issues", { team: "ENG", limit: 3 });
  expect(listed.isError).toBe(false);
  expect(Array.isArray(listed.data.issues)).toBe(true);

  // Widerruf mit Bestätigung; danach ist der Key ungültig.
  await page.getByRole("button", { name: `Revoke ${label}` }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Revoke key" }).click();
  await expect(page.getByRole("listitem", { name: `API key ${label}` })).toContainText("Revoked");
  const after = await request.post(`${API_BASE}/mcp`, {
    headers: {
      authorization: `Bearer ${token}`,
      accept: "application/json, text/event-stream",
      "content-type": "application/json",
    },
    data: { jsonrpc: "2.0", id: 1, method: "tools/list" },
  });
  expect(after.status()).toBe(401);
});
