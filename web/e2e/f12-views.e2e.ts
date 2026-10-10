import { expect, test } from "@playwright/test";

import { login, unique } from "./helpers";

// docs/08 F12: Filter setzen, als View speichern, als Favorit in der Sidebar sehen.
test("F12: save a filter as a view and mark it as a favorite", async ({ page }) => {
  await login(page, "bob");
  await page.goto("/acme/views/issues");
  await page.getByRole("button", { name: "New view" }).first().click();

  const dialog = page.getByRole("dialog");
  const name = unique("Dringende Bugs");
  await dialog.getByLabel("View name").fill(name);
  await dialog.getByRole("button", { name: "Add filter" }).click();
  await page.getByRole("menuitem", { name: "Priority" }).click();
  await page.getByRole("checkbox", { name: "Urgent" }).check();
  await page.getByRole("button", { name: "Apply" }).click();
  await expect(dialog.locator("[data-chip]")).toContainText("Priority");
  await dialog.getByRole("button", { name: "Create view" }).click();

  // Die gespeicherte View öffnet sich und trägt ihren Namen.
  await expect(page.getByRole("heading", { name })).toBeVisible();

  // In der Liste als Favorit markieren: erscheint in der Sidebar und überlebt einen Reload.
  await page.goto("/acme/views/issues");
  const row = page.getByRole("listitem").filter({ hasText: name });
  await row.getByRole("button", { name: "Add to favorites" }).click();
  const favorites = page.getByRole("region", { name: "Favorites" });
  await expect(favorites.getByRole("link", { name })).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("region", { name: "Favorites" }).getByRole("link", { name }),
  ).toBeVisible();
});
