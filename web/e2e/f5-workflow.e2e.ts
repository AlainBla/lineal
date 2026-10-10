import { expect, test, type Page } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// docs/08 F5: Issue durch den Workflow bewegen (Status-Picker `S`).
async function setStatus(page: Page, name: string): Promise<void> {
  await page.keyboard.press("s");
  const picker = page.getByRole("dialog");
  await picker.getByPlaceholder("Change status…").fill(name);
  await picker
    .getByRole("option", { name: new RegExp(`${name}$`) })
    .first()
    .click();
  await expect(picker).toBeHidden();
  await expect(page.locator('[data-property="Status"]')).toContainText(name);
}

test("F5: a new issue moves Todo, In Progress, Done and back with the S picker", async ({
  page,
}) => {
  await login(page, "bob");
  const identifier = await createIssue(page, "ENG", unique("Warenkorb vereinfachen"));
  await page.goto(`/acme/issue/${identifier}`);
  await expect(page.locator('[data-property="Status"]')).toBeVisible();

  await setStatus(page, "Todo");
  await setStatus(page, "In Progress");
  await setStatus(page, "Done");
  // Wiederöffnen
  await setStatus(page, "In Progress");

  // Der Zustand liegt auf dem Server: nach einem Reload ist er noch da.
  await page.reload();
  await expect(page.locator('[data-property="Status"]')).toContainText("In Progress");
});
