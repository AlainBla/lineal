import { expect, test } from "@playwright/test";

import { login, unique } from "./helpers";

// docs/08 F3: Issue anlegen (Modal, Taste `c`, "Create more"), Kennung lückenlos.
test("F3: three issues in a row get consecutive identifiers", async ({ page }) => {
  await login(page, "bob");
  await page.goto("/acme/team/ENG/all");
  await expect(page.getByRole("link", { name: "All issues" })).toBeVisible();

  // Taste `c` öffnet das Modal mit dem Team aus dem Kontext.
  await page.keyboard.press("c");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: "Team: ENG" })).toBeVisible();
  await dialog.getByRole("switch", { name: "Create more" }).check();

  const numbers: number[] = [];
  for (const n of [1, 2, 3]) {
    const title = unique(`Login schlägt bei SSO fehl #${n}`);
    await dialog.getByLabel("Issue title").fill(title);
    await page.keyboard.press("Control+Enter");
    const toast = page.getByText(/ENG-\d+ created/).first();
    await expect(toast).toBeVisible();
    numbers.push(Number(/ENG-(\d+)/.exec((await toast.textContent()) ?? "")?.[1]));
    // "Create more": Modal bleibt offen, Titel ist leer.
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel("Issue title")).toHaveValue("");
    // Der nächste Toast soll nicht der alte sein.
    await toast.waitFor({ state: "detached", timeout: 15_000 }).catch(() => undefined);
  }
  expect(numbers[1]).toBe(numbers[0]! + 1);
  expect(numbers[2]).toBe(numbers[1]! + 1);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});
