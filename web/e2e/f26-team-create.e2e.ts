import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// WP26 / docs/02 §2.3: Alice (Owner) legt über Settings → Teams das Team „KollegePlus“ (KP) an,
// sieht es in der Sidebar und legt dort ein Issue an (erste Kennung KP-1).
test("F26: Alice creates team KollegePlus (KP) and files KP-1", async ({ page }) => {
  await login(page, "alice");
  await page.goto("/acme/settings/teams");
  await expect(page.getByRole("heading", { name: "Teams", exact: true })).toBeVisible();
  await expect(page.getByRole("list", { name: "Teams" })).toContainText("Engineering");

  await page.getByRole("button", { name: "New team", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "New team" });
  await dialog.getByLabel("Team name").fill("KollegePlus");
  // Vorschlag aus dem Namen (ein Wort → die ersten drei Buchstaben), dann bewusst KP setzen.
  await expect(dialog.getByLabel("Team identifier")).toHaveValue("KOL");
  await dialog.getByLabel("Team identifier").fill("kp");
  await expect(dialog.getByLabel("Team identifier")).toHaveValue("KP");
  await dialog.getByRole("button", { name: "Create team" }).click();

  // Nach dem Anlegen: Team-Seite, Sidebar zeigt das neue Team ohne Reload.
  await expect(page).toHaveURL(/\/acme\/team\/KP\//);
  await expect(page.getByRole("button", { name: /\(KP\)/ })).toBeVisible();

  const identifier = await createIssue(page, "KP", unique("Erstes Issue in KollegePlus"));
  expect(identifier).toBe("KP-1");

  // Der Key ist vergeben: ein zweites Team mit KP zeigt den Konflikt des Servers.
  await page.goto("/acme/settings/teams");
  await page.getByRole("button", { name: "New team", exact: true }).click();
  const again = page.getByRole("dialog", { name: "New team" });
  await again.getByLabel("Team name").fill("Noch ein Team");
  await again.getByLabel("Team identifier").fill("KP");
  await expect(again.getByText(/already used by another team/)).toBeVisible();
});
