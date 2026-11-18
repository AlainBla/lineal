import { expect, test, type Page } from "@playwright/test";

import { login } from "./helpers";

// LIN-74: In der Listenansicht unterteilt das Sub-Grouping (zweite Gruppierungsebene) die Issues
// zusätzlich zur Hauptgruppierung. team-issues-page/GroupedIssueList rendert dafür verschachtelte
// Abschnitte mit aria-Label „<Gruppe> / <Untergruppe>".

/**
 * Wählt im hover-getriebenen HoverSelect `label` den Wert `option`. Der Trigger öffnet das
 * portalierte Listbox-Popover bei Hover; das Hinein-Hovern löscht den Close-Timer, danach ist die
 * Option stabil klickbar.
 */
async function pick(page: Page, label: string, option: string): Promise<void> {
  await page.getByRole("button", { name: label, exact: true }).hover();
  const opt = page.getByRole("option", { name: option, exact: true });
  await expect(opt).toBeVisible();
  await opt.hover();
  await opt.click();
}

test("LIN-74: sub-grouping unterteilt die Liste in eine zweite Ebene", async ({ page }) => {
  await login(page, "bob");
  await page.goto("/acme/team/ENG/all");
  await expect(page.getByRole("link", { name: "All issues" })).toBeVisible();

  // Display-Menü einmal öffnen: Hauptgruppierung Status, Untergruppierung Assignee.
  await page.getByRole("button", { name: "Display" }).click();
  await pick(page, "Grouping", "Status");
  await pick(page, "Sub-grouping", "Assignee");

  // Die Auswahl hat gegriffen: der Trigger trägt jetzt den Wert „Assignee".
  await expect(page.getByRole("button", { name: "Sub-grouping", exact: true })).toHaveText(
    /Assignee/,
  );
  await page.keyboard.press("Escape");

  // Verschachtelte Abschnitte „<Status> / <Assignee>" müssen erscheinen.
  const subSections = page.locator('section[aria-label*=" / "]');
  await expect(subSections.first()).toBeVisible({ timeout: 15_000 });
  expect(await subSections.count()).toBeGreaterThan(0);
});
