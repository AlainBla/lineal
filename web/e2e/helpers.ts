import { expect, type Page } from "@playwright/test";

/** Erfundene Seed-User (`lineal seed`), alle mit dem Seed-Passwort. */
export type Who = "alice" | "bob" | "carol";
export const PASSWORD = "lineal-dev";
export const WORKSPACE = "acme";

/** Meldet über das echte Login-Formular an und wartet auf die App-Shell. */
export async function login(page: Page, who: Who): Promise<void> {
  await page.goto("/login");
  await page.getByLabel("Email").fill(`${who}@acme.example`);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Log in" }).click();
  // Nach dem Login landet man auf der Startseite der Shell.
  await expect(page.getByText(/^Welcome, /)).toBeVisible();
}

/** Zufälliger Zusatz für Titel, damit wiederholte Läufe sich nicht überschneiden. */
export function unique(prefix: string): string {
  return `${prefix} ${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * Legt über das Create-Modal (Taste `c`) ein Issue in `team` an und liefert die Kennung aus dem
 * Toast ("ENG-123 created").
 */
export async function createIssue(page: Page, team: string, title: string): Promise<string> {
  await page.goto(`/${WORKSPACE}/team/${team}/all`);
  await expect(page.getByRole("link", { name: "All issues" })).toBeVisible();
  await page.keyboard.press("c");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: `Team: ${team}` })).toBeVisible();
  await dialog.getByLabel("Issue title").fill(title);
  await page.keyboard.press("Control+Enter");
  const toast = page.getByText(new RegExp(`${team}-\\d+ created`)).first();
  await expect(toast).toBeVisible();
  const identifier = /([A-Z]+-\d+)/.exec((await toast.textContent()) ?? "")?.[1];
  if (!identifier) throw new Error("no identifier in toast");
  await expect(dialog).toBeHidden();
  return identifier;
}
