import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// WP33: Mehrfachauswahl in der Liste und Bulk-Statuswechsel.
test("F33: three selected issues move to In Progress with one bulk action", async ({ page }) => {
  await login(page, "bob");
  const identifiers: string[] = [];
  for (const prefix of ["Bulk A", "Bulk B", "Bulk C"]) {
    identifiers.push(await createIssue(page, "ENG", unique(prefix)));
  }

  await page.goto("/acme/team/ENG/all");
  await expect(page.getByRole("link", { name: "All issues" })).toBeVisible();
  for (const id of identifiers) {
    await page.getByRole("checkbox", { name: `Select ${id}` }).click();
  }
  const bar = page.getByRole("toolbar", { name: "Bulk actions" });
  await expect(bar.getByText("3 selected")).toBeVisible();

  await bar.getByRole("button", { name: "Bulk Status" }).click();
  const picker = page.getByRole("dialog");
  await picker.getByPlaceholder("Change status…").fill("In Progress");
  await picker.getByRole("option", { name: /In Progress$/ }).click();

  await expect(page.getByText("Updated 3 issues")).toBeVisible();
  await expect(bar).toBeHidden();
  const progress = page.getByRole("region", { name: "In Progress" });
  for (const id of identifiers) await expect(progress.getByText(id)).toBeVisible();

  // Der Zustand liegt auf dem Server.
  await page.reload();
  const reloaded = page.getByRole("region", { name: "In Progress" });
  for (const id of identifiers) await expect(reloaded.getByText(id)).toBeVisible();
});
