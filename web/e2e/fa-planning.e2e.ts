import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// F-A: Projekt mit Meilenstein per UI anlegen (F.01) und einem Issue zuordnen (F.07);
// Issue löschen und in „Recently deleted“ wiederherstellen (F.04).
test("F.01/F.07: project with a milestone is created in the UI and an issue is assigned to it", async ({
  page,
}) => {
  await login(page, "alice");
  const projectName = unique("Planung");
  const issueTitle = unique("Meilenstein-Issue");

  // Projekt über den Dialog anlegen.
  await page.goto("/acme/projects/all");
  await page.getByRole("button", { name: "New project" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Project name").fill(projectName);
  await dialog.getByRole("checkbox", { name: /Engineering/ }).check();
  await dialog.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByRole("heading", { name: projectName })).toBeVisible();
  await expect(page.getByLabel("Project name")).toHaveValue(projectName);

  // Eigenschaften bearbeiten: Status.
  await page.getByLabel("Status", { exact: true }).selectOption("started");
  await expect(page.getByLabel("Status", { exact: true })).toHaveValue("started");

  // Meilenstein anlegen.
  await page.getByRole("link", { name: "Milestones" }).click();
  await page.getByLabel("Milestone name", { exact: true }).fill("Beta");
  await page.getByLabel("Target date", { exact: true }).fill("2027-01-31");
  await page.getByRole("button", { name: "Add milestone" }).click();
  await expect(page.getByLabel("Milestone name Beta")).toBeVisible();

  // Issue anlegen und Projekt sowie Meilenstein setzen (Shift+P, Shift+M).
  const identifier = await createIssue(page, "ENG", issueTitle);
  await page.goto(`/acme/issue/${identifier}`);
  await expect(page.getByLabel("Title")).toHaveValue(issueTitle);
  await page.keyboard.press("Shift+P");
  await page.getByPlaceholder("Add to project…").fill(projectName);
  await page.getByRole("option", { name: projectName }).click();
  const sidebar = page.getByLabel("Properties");
  await expect(sidebar.getByText(projectName)).toBeVisible();
  await page.keyboard.press("Shift+M");
  await page.getByPlaceholder("Set milestone…").fill("Beta");
  await page.getByRole("option", { name: /Beta/ }).click();
  await expect(sidebar.locator('[data-property="Milestone"]').getByText("Beta")).toBeVisible();

  // Der Zustand liegt auf dem Server; der Issues-Tab des Projekts gruppiert nach Meilenstein.
  await page.reload();
  await expect(sidebar.locator('[data-property="Milestone"]').getByText("Beta")).toBeVisible();
  await page.goto("/acme/projects/all");
  await page.getByRole("link", { name: projectName }).click();
  await page
    .getByRole("navigation", { name: "Sections" })
    .getByRole("link", { name: "Issues" })
    .click();
  await expect(page.getByText(issueTitle)).toBeVisible();
});

test("F.04: a deleted issue appears in Recently deleted and can be restored", async ({ page }) => {
  await login(page, "bob");
  const title = unique("Papierkorb");
  const identifier = await createIssue(page, "ENG", title);

  // Löschen über die Bulk-Leiste.
  await page.goto("/acme/team/ENG/all");
  await page.getByRole("checkbox", { name: `Select ${identifier}` }).click();
  await page.getByRole("button", { name: "Bulk delete" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByText(title)).toBeHidden();

  // Teamweiter Papierkorb.
  await page.goto("/acme/team/ENG/trash");
  await expect(page.getByText(title)).toBeVisible();
  await page.getByRole("button", { name: `Restore ${title}` }).click();
  await expect(page.getByText(title)).toBeHidden();

  // Wieder in der Liste; im Workspace-Papierkorb nicht mehr.
  await page.goto("/acme/team/ENG/all");
  await expect(page.getByText(title)).toBeVisible();
  await page.goto("/acme/trash/issues");
  await expect(page.getByText(title)).toBeHidden();
});
