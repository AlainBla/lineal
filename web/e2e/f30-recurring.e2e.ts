import { expect, test } from "@playwright/test";

import { login, sql, unique, WORKSPACE } from "./helpers";

// WP30 / docs/03 §13: Alice legt im Team ENG ein Issue-Template und daraus ein wiederkehrendes
// Issue an. Die Uhr wird per Test-Hook (SQL) vorgestellt: `next_run_at` liegt in der
// Vergangenheit, der Worker-Job `recurring_issues` (läuft minütlich) legt dann das Issue an.
test("F30: a recurring issue is created by the job when it is due", async ({ page }) => {
  test.setTimeout(150_000);
  await login(page, "alice");

  // 1. Issue-Template im Team (Titel, Sub-Issue, Fälligkeit).
  const title = unique("Wöchentlicher Report");
  await page.goto(`/${WORKSPACE}/settings/teams/ENG/templates`);
  await page.getByRole("button", { name: /New template/ }).click();
  await page.getByLabel("Template name").fill(title);
  await page.getByLabel("Title", { exact: true }).fill(title);
  await page.getByLabel("Due in days").fill("3");
  await page.getByRole("button", { name: /Add sub-issue/ }).click();
  await page.getByLabel("Sub-issue 1 title").fill("Zahlen sammeln");
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("button", { name: `Edit ${title}` })).toBeVisible();

  // 2. Wiederkehrendes Issue anlegen (wöchentlich, Montag 09:00 in der Team-Zeitzone).
  await page.goto(`/${WORKSPACE}/settings/teams/ENG/recurring`);
  await page.getByRole("button", { name: /New recurring issue/ }).click();
  await page.getByLabel("Issue template").selectOption({ label: title });
  await page.getByLabel("Weekday").selectOption("1");
  await page.getByLabel("Time of day").fill("09:00");
  await page.getByRole("button", { name: "Create recurring issue" }).click();
  const row = page.getByRole("listitem", { name: `Recurring issue ${title}` });
  await expect(row).toContainText("Every week on Monday at 09:00");
  await expect(row).toContainText("Next:");

  // 3. Uhr vorstellen: der nächste Lauf ist fällig.
  sql(
    `UPDATE recurring_issue SET next_run_at = now() - interval '1 minute' ` +
      `WHERE template_id IN (SELECT id FROM template WHERE name = '${title}')`,
  );

  // 4. Der Worker erzeugt das Issue (Job läuft einmal pro Minute); die Regel zeigt den Lauf.
  await expect(async () => {
    await page.goto(`/${WORKSPACE}/settings/teams/ENG/recurring`);
    await expect(row).toContainText("last created", { timeout: 2_000 });
  }).toPass({ timeout: 120_000, intervals: [5_000] });
  await expect(row.getByRole("alert")).toHaveCount(0);

  // 5. Das Issue samt Sub-Issue steht in der Team-Liste.
  await page.goto(`/${WORKSPACE}/team/ENG/all`);
  await expect(page.getByText(title, { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Zahlen sammeln").first()).toBeVisible();

  // 6. Pausieren und löschen.
  await page.goto(`/${WORKSPACE}/settings/teams/ENG/recurring`);
  await page.getByRole("button", { name: `Pause ${title}` }).click();
  await expect(row).toContainText("Paused");
  await page.getByRole("button", { name: `Delete ${title}` }).click();
  await page.getByRole("button", { name: `Confirm delete ${title}` }).click();
  await expect(row).toHaveCount(0);
});
