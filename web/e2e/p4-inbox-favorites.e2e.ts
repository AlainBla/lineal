import { expect, test, type Page } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// Paket 4 (B4.01): Ein Klick auf eine Issue-Benachrichtigung bleibt in der Inbox (Split-Ansicht).
test("P4: opening an issue notification keeps the inbox list and URL", async ({ browser }) => {
  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");

  const identifier = await createIssue(alice, "ENG", unique("Inbox bleibt"));
  await alice.goto(`/acme/issue/${identifier}`);
  const editor = alice.locator('[aria-label="Activity"] [role="textbox"]').last();
  await editor.click();
  await alice.keyboard.type("Bitte schauen @Bo");
  await alice.getByRole("option", { name: "Bob" }).click();
  await alice.keyboard.type(" kurz drauf");
  await alice.getByRole("button", { name: "Comment", exact: true }).click();
  await expect(alice.getByLabel("Comment by Alice")).toBeVisible();

  await bob.goto("/acme/inbox");
  const entry = bob.getByRole("option").filter({ hasText: identifier });
  await expect(async () => {
    await bob.reload();
    await expect(entry).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 40_000 });

  await entry.click();
  // Detail eingebettet: Titelfeld sichtbar, URL und Liste unverändert.
  await expect(bob.getByLabel("Title")).toBeVisible();
  await expect(bob).toHaveURL(/\/acme\/inbox$/);
  await expect(bob.getByRole("listbox", { name: "Notifications" })).toBeVisible();
  await bob.keyboard.press("j");
  await expect(bob).toHaveURL(/\/acme\/inbox$/);

  await aliceCtx.close();
  await bobCtx.close();
});

async function createFavoriteView(page: Page, name: string): Promise<void> {
  await page.goto("/acme/views/issues");
  await page.getByRole("button", { name: "New view" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("View name").fill(name);
  await dialog.getByRole("button", { name: "Create view" }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
  // Die URL trägt den Namens-Slug (B4.20).
  await expect(page).toHaveURL(/\/acme\/view\/[a-z0-9-]+-[0-9a-f]{12}$/);
  await page.goto("/acme/views/issues");
  const row = page.getByRole("listitem").filter({ hasText: name });
  await row.getByRole("button", { name: "Add to favorites" }).click();
  await expect(
    page.getByRole("region", { name: "Favorites" }).getByRole("link", { name }),
  ).toBeVisible();
}

// Paket 4 (B4.02): Favoriten lassen sich per Maus umsortieren.
test("P4: favorites can be reordered with the mouse and keep the order after reload", async ({
  page,
}) => {
  await login(page, "bob");
  const first = unique("Fav A");
  const second = unique("Fav B");
  await createFavoriteView(page, first);
  await createFavoriteView(page, second);

  const links = () => page.getByRole("region", { name: "Favorites" }).getByRole("link");
  await expect(links().filter({ hasText: first })).toBeVisible();
  const names = async () => (await links().allTextContents()).map((t) => t.trim());
  const before = await names();
  expect(before.indexOf(first)).toBeLessThan(before.indexOf(second));

  // B vor A ziehen: Zeile greifen (nicht nur den Griff) und nach oben bewegen.
  const a = await links().filter({ hasText: first }).boundingBox();
  const b = await links().filter({ hasText: second }).boundingBox();
  if (!a || !b) throw new Error("no bounding boxes");
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2 - 10, { steps: 4 });
  await page.mouse.move(a.x + a.width / 2, a.y + 2, { steps: 8 });
  await page.mouse.up();

  await expect(async () => {
    const after = await names();
    expect(after.indexOf(second)).toBeLessThan(after.indexOf(first));
  }).toPass({ timeout: 10_000 });
  await page.reload();
  const reloaded = await names();
  expect(reloaded.indexOf(second)).toBeLessThan(reloaded.indexOf(first));
});
