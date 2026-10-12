import { expect, test, type Page } from "@playwright/test";

import { PASSWORD, sql } from "./helpers";

// WP29: Account-Settings. Ein eigener Nutzer (Dana) mit zwei Browser-Sessions ändert sein Passwort;
// danach ist die andere Session ungültig, die eigene bleibt, und nur das neue Passwort meldet an.
// Dana wird per SQL aus dem Seed-Hash von Bob angelegt, damit die Seed-Nutzer der anderen Flows
// (parallele Tests) ihr Passwort behalten.
const EMAIL = "dana@acme.example";
const NEW_PASSWORD = "dana-new-password-1";

async function loginAs(page: Page, email: string, password: string): Promise<void> {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Log in" }).click();
}

test("F29: changing the password signs out the other session", async ({ browser }) => {
  sql(
    `INSERT INTO "user" (organization_id, name, display_name, email, role, password_hash)
     SELECT organization_id, 'Dana Example', 'dana', '${EMAIL}', 'user', password_hash
     FROM "user" WHERE email = 'bob@acme.example'
     AND NOT EXISTS (SELECT 1 FROM "user" WHERE email = '${EMAIL}')`,
  );
  const here = await browser.newContext();
  const other = await browser.newContext();
  const a = await here.newPage();
  const b = await other.newPage();
  await loginAs(a, EMAIL, PASSWORD);
  await expect(a.getByText(/^Welcome, /)).toBeVisible();
  await loginAs(b, EMAIL, PASSWORD);
  await expect(b.getByText(/^Welcome, /)).toBeVisible();

  // Profil: Name ändern.
  await a.goto("/acme/settings/account/profile");
  await expect(a.getByRole("link", { name: "Profile" })).toHaveAttribute("aria-current", "page");
  await a.getByLabel("Full name").fill("Dana Beispiel");
  await a.getByRole("button", { name: "Save", exact: true }).click();
  await expect(a.getByText("Profile saved")).toBeVisible();

  // Passwort & Sessions: zwei aktive Sessions, eine davon „This device“.
  await a.goto("/acme/settings/account/security");
  const sessions = a.getByRole("list", { name: "Active sessions" });
  await expect(sessions.getByRole("listitem")).toHaveCount(2);
  await expect(sessions).toContainText("This device");

  // Falsches altes Passwort: Fehlermeldung, die andere Session bleibt gültig.
  await a.getByLabel("Current password").fill("falsches-passwort");
  await a.getByLabel("New password", { exact: true }).fill(NEW_PASSWORD);
  await a.getByLabel("Repeat new password").fill(NEW_PASSWORD);
  await a.getByRole("button", { name: "Change password" }).click();
  await expect(a.getByText("The current password is incorrect.")).toBeVisible();
  await b.goto("/acme/settings/account/profile");
  await expect(b.getByLabel("Full name")).toBeVisible();

  // Richtiges altes Passwort: Erfolg, die andere Session wird abgemeldet.
  await a.getByLabel("Current password").fill(PASSWORD);
  await a.getByRole("button", { name: "Change password" }).click();
  await expect(a.getByText(/Password changed\. 1 other session was signed out\./)).toBeVisible();
  await expect(sessions.getByRole("listitem")).toHaveCount(1);

  // Die alte Session ist ungültig (zurück zum Login), die eigene bleibt angemeldet.
  await b.reload();
  await expect(b).toHaveURL(/\/login/);
  await a.reload();
  await expect(a.getByRole("list", { name: "Active sessions" })).toBeVisible();

  // Nur das neue Passwort meldet an.
  const fresh = await browser.newContext();
  const c = await fresh.newPage();
  await loginAs(c, EMAIL, PASSWORD);
  await expect(c).toHaveURL(/\/login/);
  await loginAs(c, EMAIL, NEW_PASSWORD);
  await expect(c.getByText(/^Welcome, /)).toBeVisible();

  await Promise.all([here.close(), other.close(), fresh.close()]);
});
