import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// docs/08 F7: Issue in ein anderes Team verschieben (Strg+Umschalt+M).
test("F7: moving an issue to another team gives it a new identifier and keeps the old link", async ({
  page,
}) => {
  await login(page, "bob");
  const oldId = await createIssue(page, "ENG", unique("Falsches Team"));
  await page.goto(`/acme/issue/${oldId}`);
  await expect(page.locator('[data-property="Team"]')).toContainText("ENG");

  await page.keyboard.press("Control+Shift+M");
  const picker = page.getByRole("dialog");
  await picker.getByPlaceholder("Move to team…").fill("Web");
  await page.keyboard.press("Enter");
  // Ohne Labels/Projekt gibt es nichts zu bestätigen: der Wechsel läuft direkt.

  const toast = page.getByText(/Moved to WEB-\d+/);
  await expect(toast).toBeVisible();
  const newId = /(WEB-\d+)/.exec((await toast.textContent()) ?? "")?.[1];
  expect(newId).toBeTruthy();

  // Neue Kennung, Team WEB; der alte Link löst weiter auf.
  await page.goto(`/acme/issue/${newId}`);
  await expect(page.locator('[data-property="Team"]')).toContainText("WEB");
  await page.goto(`/acme/issue/${oldId}`);
  await expect(page.locator('[data-property="Team"]')).toContainText("WEB");
});
