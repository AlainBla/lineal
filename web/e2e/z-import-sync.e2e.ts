import { expect, test } from "@playwright/test";

import { login } from "./helpers";

/**
 * Import und regelmäßiger Abgleich gegen den lokalen Mock-Linear-Server
 * (`crates/lineal-import/examples/mock_linear.rs`, erfundene Daten; der Worker liest den Endpunkt
 * aus `LINEAL_IMPORT_LINEAR_ENDPOINT`). Nie das echte Linear.
 */
test("import from the mock Linear, then sync, merge a placeholder and join the team", async ({
  page,
}) => {
  test.setTimeout(300_000);
  await login(page, "alice");
  await page.goto("/acme/settings/import");

  // Verbindung prüfen und nur das Team PLT übernehmen (die Demo-Daten kennen ENG, WEB, DES schon).
  await page.getByLabel("Linear API key", { exact: true }).fill("lin_api_e2e_mock_key_0000000000");
  await page.getByRole("button", { name: "Check connection" }).click();
  await expect(page.getByLabel("Import team PLT")).toBeVisible({ timeout: 90_000 });
  for (const key of ["ENG", "WEB", "DES"]) await page.getByLabel(`Import team ${key}`).uncheck();
  await page.getByRole("button", { name: "Start import" }).click();
  await expect(page.getByTestId("run-status")).toHaveText("Completed", { timeout: 180_000 });

  // Platzhalter erscheinen im Bericht nicht als Fehler; der Admin tritt den importierten Teams bei.
  const join = page.getByRole("button", { name: /Join imported teams/ });
  await expect(join).toBeVisible();
  await join.click();
  await expect(join).toBeHidden();
  await expect(page.getByRole("link", { name: "Browse teams" })).toBeVisible();
  await expect(
    page.getByRole("complementary", { name: "Sidebar" }).getByText("Platform"),
  ).toBeVisible();

  // Sync einrichten (Schlüssel nur senden, nie anzeigen) und sofort abgleichen.
  const sync = page.getByTestId("sync-section");
  await sync.getByLabel("Linear API key for the sync").fill("lin_api_e2e_mock_key_0000000000");
  await sync.getByLabel("Sync interval").selectOption("hourly");
  await sync.getByRole("button", { name: "Set up sync" }).click();
  await expect(sync.getByTestId("sync-state")).toHaveText("Active");
  await expect(sync.getByTestId("sync-next")).not.toHaveText("–");
  await expect(page.locator("body")).not.toContainText("e2e_mock_key");
  await sync.getByRole("button", { name: "Sync now" }).click();
  await expect(sync.getByTestId("sync-last")).toContainText("Completed", { timeout: 180_000 });
  await sync.getByRole("button", { name: "Pause" }).click();
  await expect(sync.getByTestId("sync-state")).toHaveText("Paused");

  // Platzhalter (andere E-Mail in Linear) einem vorhandenen Mitglied zuordnen.
  await page.goto("/acme/settings/members");
  await expect(page.getByText("dave@acme.test")).toBeVisible();
  await page.getByLabel("Filter members").selectOption("imported");
  await page.getByRole("button", { name: "Merge dave@acme.test with an existing member" }).click();
  await page.getByLabel("Merge into").selectOption({ label: "bob (bob@acme.example)" });
  await page.getByRole("button", { name: "Merge", exact: true }).click();
  await expect(page.getByText("dave@acme.test")).toBeHidden();
});
