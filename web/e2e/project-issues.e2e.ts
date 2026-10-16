import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// Projekt-Issues (docs/08 §2.7): Filter setzen, Gruppierung ändern, Seitenpanel ein/aus.
test("project issues: filter, grouping and the side panel", async ({ page }) => {
  await login(page, "alice");
  const projectName = unique("Ansicht");
  const issueTitle = unique("Projekt-Issue");

  // Projekt mit Issue anlegen.
  await page.goto("/acme/projects/all");
  await page.getByRole("button", { name: "New project" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Project name").fill(projectName);
  await dialog.getByRole("checkbox", { name: /Engineering/ }).check();
  await dialog.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByRole("heading", { name: projectName })).toBeVisible();
  const projectUrl = new URL(page.url()).pathname.replace(/\/[^/]+$/, "");

  const identifier = await createIssue(page, "ENG", issueTitle);
  await page.goto(`/acme/issue/${identifier}`);
  await expect(page.getByLabel("Title")).toHaveValue(issueTitle);
  await page.keyboard.press("Shift+P");
  await page.getByPlaceholder("Add to project…").fill(projectName);
  await page.getByRole("option", { name: projectName }).click();
  await expect(page.getByLabel("Properties").getByText(projectName)).toBeVisible();

  // Issues-Tab: Seitenpanel mit Properties, Milestones und Progress.
  await page.goto(`${projectUrl}/issues`);
  await expect(page.getByText(issueTitle)).toBeVisible();
  const panel = page.getByRole("complementary", { name: "Project details" });
  await expect(panel.getByRole("region", { name: "Properties" })).toBeVisible();
  await expect(panel.getByRole("region", { name: "Progress" })).toContainText("Scope");

  // Meilenstein im Panel anlegen.
  await panel.getByRole("button", { name: "Add milestone" }).click();
  await panel.getByLabel("Milestone name").fill("Beta");
  await panel.getByRole("button", { name: "Add", exact: true }).click();
  await expect(panel.getByText("Beta")).toBeVisible();

  // Gruppierung ändern: nach Meilenstein, dann nach Priorität.
  await page.getByRole("button", { name: "Display options" }).click();
  const display = page.getByRole("dialog", { name: "Display options" });
  await display.getByLabel("Grouping", { exact: true }).selectOption("milestone");
  await expect(page.getByRole("region", { name: "No milestone" })).toBeVisible();
  await display.getByLabel("Grouping", { exact: true }).selectOption("priority");
  await expect(page.getByRole("region", { name: "No priority" })).toBeVisible();
  await page.keyboard.press("Escape");
  // Abweichung vom Standard: Punkt am Anzeige-Knopf.
  await expect(page.getByTestId("display-customized")).toBeVisible();

  // Filter: Priorität „Urgent“ blendet das Issue ohne Priorität aus, „No priority“ zeigt es.
  await page.getByRole("button", { name: "Add filter" }).click();
  await page.getByRole("menuitem", { name: "Priority", exact: true }).click();
  await page.getByRole("checkbox", { name: "Urgent" }).check();
  await page.getByRole("button", { name: "Apply" }).click();
  await expect(page.getByText("No matching issues")).toBeVisible();
  await page.getByRole("button", { name: "Remove filter Priority" }).click();
  await expect(page.getByText(issueTitle)).toBeVisible();

  // Seitenpanel aus und (nach Reload) weiterhin aus; wieder ein.
  await page.getByRole("button", { name: "Hide side panel" }).click();
  await expect(panel).toBeHidden();
  await page.reload();
  await expect(page.getByText(issueTitle)).toBeVisible();
  await expect(panel).toBeHidden();
  await page.getByRole("button", { name: "Show side panel" }).click();
  await expect(panel).toBeVisible();

  // Die Gruppierung wurde je Projekt gespeichert und gilt auch nach dem Reload.
  await expect(page.getByRole("region", { name: "No priority" })).toBeVisible();
});
