import { expect, test } from "@playwright/test";

import { createIssue, login, sql, unique } from "./helpers";

// 1x1-PNG.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
).toString("base64");

async function createProject(page: import("@playwright/test").Page, name: string) {
  await page.goto("/acme/projects/all");
  await page.getByRole("button", { name: "New project" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Project name").fill(name);
  await dialog.getByRole("checkbox", { name: /Engineering/ }).check();
  await dialog.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
}

// LIN-25: Detailseite des Meilensteins.
test("LIN-25: milestone page keeps description, image, comment and issue in one place", async ({
  page,
}) => {
  await login(page, "alice");
  await createProject(page, unique("Meilenstein"));
  await page.getByRole("main").getByRole("link", { name: "Milestones" }).click();
  await page.getByLabel("Milestone name", { exact: true }).fill("Abnahmetest");
  await page.getByRole("button", { name: "Add milestone" }).click();
  await page.getByRole("link", { name: "Abnahmetest" }).click();
  await expect(page.getByRole("heading", { name: "Abnahmetest", exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\/project\/[^/]+\/milestone\/[0-9a-f-]+$/);
  await expect(page.getByText("0 of 0 done")).toBeVisible();

  // Beschreibung mit Bild; beides überlebt das Neuladen.
  const editor = page.getByRole("textbox", { name: "Description" });
  await editor.click();
  await page.keyboard.type("Rollback per DNS.");
  await page.evaluate(async (b64) => {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const data = new DataTransfer();
    data.items.add(new File([bytes], "pixel.png", { type: "image/png" }));
    document
      .querySelector('[aria-label="Description"][role="textbox"]')!
      .dispatchEvent(
        new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }),
      );
  }, PNG);
  await expect(editor.locator("img[alt='pixel.png']")).toBeVisible();
  // Der Text läuft über die Collab-Sitzung und ist nach dem Speichern im Raum wieder da.
  await expect(async () => {
    await page.reload();
    const saved = page.getByRole("textbox", { name: "Description" });
    await expect(saved).toContainText("Rollback per DNS", { timeout: 3000 });
    await expect(saved.locator("img[alt='pixel.png']")).toHaveAttribute("src", /^\/files\//, {
      timeout: 3000,
    });
  }).toPass({ timeout: 20_000 });

  // Kommentar am Meilenstein.
  const activity = page.getByRole("region", { name: "Activity" });
  await activity.getByRole("textbox").click();
  await page.keyboard.type("Freigabe liegt vor.");
  await page.keyboard.press("Control+Enter");
  await expect(activity.getByText("Freigabe liegt vor.")).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("region", { name: "Activity" }).getByText("Freigabe liegt vor."),
  ).toBeVisible();

  // „New issue“ belegt Projekt und Meilenstein vor.
  const title = unique("Redirects");
  await page.getByRole("button", { name: "New issue", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Issue title").fill(title);
  await page.keyboard.press("Control+Enter");
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("region", { name: "Issues" }).getByText(title)).toBeVisible();
  await expect(page.getByText("0 of 1 done")).toBeVisible();

  // Ressourcen: Link, Datei und neues Dokument bleiben am Meilenstein.
  const resources = page.getByRole("region", { name: "Resources" });
  await resources.getByRole("button", { name: "Add resource" }).click();
  await page.getByRole("menuitem", { name: "Add link" }).click();
  await resources.getByLabel("Link URL").fill("https://example.com/lighthouse");
  await resources.getByLabel("Link title").fill("Lighthouse-Bericht");
  await resources.getByRole("button", { name: "Add", exact: true }).click();
  await expect(resources.getByRole("link", { name: "Lighthouse-Bericht" })).toBeVisible();

  await resources.getByLabel("Upload file").setInputFiles({
    name: "freigabe.png",
    mimeType: "image/png",
    buffer: Buffer.from(PNG, "base64"),
  });
  const file = resources.getByRole("link", { name: "freigabe.png" });
  await expect(file).toBeVisible();
  const href = await file.getAttribute("href");
  expect((await page.request.get(href!)).status()).toBe(200);

  await resources.getByRole("button", { name: "Add resource" }).click();
  await page.getByRole("menuitem", { name: "New document" }).click();
  await resources.getByLabel("Document title").fill("Checkliste");
  await resources.getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(/\/document\//);
  await page.goBack();
  await expect(
    page.getByRole("region", { name: "Resources" }).getByRole("link", { name: "Checkliste" }),
  ).toBeVisible();

  // Terminverlauf: Das Datum zu verschieben bleibt sichtbar.
  await page.getByLabel("Target date", { exact: true }).fill("2027-03-01");
  await expect(page.getByLabel("Target date", { exact: true })).toHaveValue("2027-03-01");
  await page.reload();
  await page.getByLabel("Target date", { exact: true }).fill("2027-03-15");
  await expect(page.getByText(/moved once \(\+14 days\)/)).toBeVisible();
  await expect(page.getByRole("region", { name: "Target date history" })).toBeVisible();

  // Abschließen: offene Issues bleiben, die Liste friert ein, die Notiz bleibt lesbar.
  await page.getByRole("button", { name: "Complete milestone" }).click();
  const complete = page.getByRole("dialog");
  await complete.getByLabel("Keep here").check();
  await complete.getByLabel("Result note").fill("Live seit heute.");
  await complete.getByRole("button", { name: "Complete milestone" }).click();
  await expect(complete).toBeHidden();
  await expect(page.getByText("Completed", { exact: true })).toBeVisible();
  await expect(page.getByText("Live seit heute.")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Issues at completion" }).getByText(title),
  ).toBeVisible();

  // Wiederöffnen: die Notiz bleibt, die Liste ist wieder live.
  await page.getByRole("button", { name: "Reopen" }).click();
  await expect(page.getByRole("button", { name: "Complete milestone" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Issues" }).getByText(title)).toBeVisible();
  await expect(page.getByText("Live seit heute.")).toBeVisible();

  // Alle Meilensteine: über die Navigation erreichbar, die Zeile führt zurück.
  await page.getByRole("link", { name: "Milestones", exact: true }).first().click();
  await expect(page).toHaveURL(/\/acme\/milestones$/);
  await page.getByRole("link", { name: "Abnahmetest" }).first().click();
  await expect(page).toHaveURL(/\/milestone\//);

  // Umwandeln: Issue und Ressourcen wandern ins neue Projekt, der Meilenstein entfällt.
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Milestone actions Abnahmetest" }).click();
  await page.getByRole("menuitem", { name: /Convert to project/ }).click();
  await expect(page).toHaveURL(/\/project\/[^/]+\/overview$/);
  await expect(page.getByRole("heading", { name: "Abnahmetest" })).toBeVisible();
  await page
    .getByRole("navigation", { name: "Sections" })
    .getByRole("link", { name: "Issues" })
    .click();
  await expect(page.getByText(title)).toBeVisible();
});

// LIN-25 Stufe 4: Raute ziehen, im Statusbericht erwähnen, im Kalender abonnieren.
test("LIN-25: drag a milestone on the roadmap, mention it in an update, subscribe in a calendar", async ({
  page,
}) => {
  await login(page, "alice");
  const project = unique("Reichweite");
  const name = unique("Rollout");
  await createProject(page, project);
  const updates = page.url().replace(/\/[^/]+$/, "/updates");
  await page.getByRole("main").getByRole("link", { name: "Milestones" }).click();
  await page.getByLabel("Milestone name", { exact: true }).fill(name);
  await page.getByRole("button", { name: "Add milestone" }).click();
  await expect(page.getByRole("link", { name })).toBeVisible();
  sql(
    `UPDATE project SET start_date = '2026-11-02', target_date = '2026-12-18' WHERE name = '${project}'`,
  );
  sql(`UPDATE project_milestone SET target_date = '2026-11-20' WHERE name = '${name}'`);

  // Zoom „Month“ = 7 px je Tag: 70 px = 10 Tage nach rechts. Das Ziehen navigiert nicht.
  await page.goto("/acme/roadmap");
  const marker = page.getByRole("gridcell").locator(`a[title^="${name}"]`);
  await expect(marker).toHaveAttribute("title", `${name} · Nov 20, 2026`);
  await marker.scrollIntoViewIfNeeded();
  const box = (await marker.boundingBox())!;
  const saved = page.waitForResponse(
    (r) =>
      r.url().endsWith("/graphql") &&
      (r.request().postData() ?? "").includes("PlanningMilestoneMove"),
  );
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 35, box.y + box.height / 2, { steps: 5 });
  await page.mouse.move(box.x + box.width / 2 + 70, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  expect(((await (await saved).json()) as { errors?: unknown }).errors).toBeUndefined();
  await expect(marker).toHaveAttribute("title", `${name} · Nov 30, 2026`);
  await expect(page).toHaveURL(/\/acme\/roadmap$/);

  // Ein Klick öffnet weiterhin den Meilenstein, der Terminverlauf kennt die Verschiebung.
  await page.reload();
  await marker.click();
  await expect(page).toHaveURL(/\/milestone\/[0-9a-f-]+$/);
  await expect(page.getByLabel("Target date", { exact: true })).toHaveValue("2026-11-30");
  await expect(page.getByRole("region", { name: "Target date history" })).toContainText(
    "30 Nov 2026",
  );
  const milestonePage = page.url();

  // Erwähnung im Statusbericht.
  await page.goto(updates);
  const form = page.getByRole("form", { name: "Post update" });
  await form.getByLabel("Update text").fill("Termin verschoben: ");
  await form.getByRole("button", { name: "Mention milestone" }).click();
  await page.getByRole("menuitem", { name }).click();
  await expect(form.getByLabel("Update text")).toHaveValue(new RegExp(`\\[${name}\\]\\(http`));
  await form.getByRole("button", { name: "Post update" }).click();
  await expect(form.getByLabel("Update text")).toHaveValue("");
  await page.goto(milestonePage);
  const mentions = page.getByRole("region", { name: "Status updates" });
  await expect(mentions).toContainText("Termin verschoben");
  await expect(mentions.getByRole("link", { name })).toHaveAttribute(
    "href",
    new RegExp(`${new URL(milestonePage).pathname}$`),
  );

  // Kalender-Abo: Der Link funktioniert ohne Anmeldung und lässt sich abschalten.
  await page.goto("/acme/milestones");
  await page.getByRole("button", { name: "Calendar" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: /Create (new )?link/ }).click();
  const link = await dialog.getByLabel("Calendar link").inputValue();
  const path = new URL(link).pathname;
  const anon = await page.request.get(path, { headers: { cookie: "" } });
  expect(anon.status()).toBe(200);
  expect(anon.headers()["content-type"]).toContain("text/calendar");
  const ics = (await anon.text()).replace(/\r\n /g, "");
  expect(ics).toContain(`SUMMARY:${name} (${project})`);
  expect(ics).toContain("DTSTART;VALUE=DATE:20261130");
  await dialog.getByRole("button", { name: "Turn off" }).click();
  await expect(dialog.getByRole("button", { name: "Create link" })).toBeVisible();
  expect((await page.request.get(path)).status()).toBe(404);
});

// LIN-26: Projekt-Verweise im Text sind Chips und öffnen das Projekt.
test("LIN-26: [P-…] in an issue description becomes a chip that opens the project", async ({
  page,
}) => {
  await login(page, "alice");
  const name = unique("Chip-Projekt");
  await createProject(page, name);
  const identifier = /\/project\/([^/]+)\//.exec(page.url())![1]!;
  const issue = await createIssue(page, "ENG", unique("Verweis"));
  await page.goto(`/acme/issue/${issue}`);
  const editor = page.getByRole("textbox", { name: "Description" });
  await editor.click();
  await page.keyboard.type(`Siehe [${identifier}] dazu`);
  const chip = editor.locator(".issue-ref-title", { hasText: name });
  await expect(chip).toBeVisible();
  await expect(editor.locator(".issue-ref-bracket").first()).toBeHidden();
  await chip.click();
  await expect(page).toHaveURL(new RegExp(`/project/${identifier}/overview$`));
});
