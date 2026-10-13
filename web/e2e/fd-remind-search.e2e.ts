import { expect, test } from "@playwright/test";

import { createIssue, login, sql, unique } from "./helpers";

// F.10: Remind me setzen -> Job (Zeit per SQL vorgestellt) -> Eintrag in der eigenen Inbox.
test("F10: Remind me on an issue ends up in the inbox once the reminder is due", async ({
  page,
}) => {
  test.setTimeout(150_000);
  await login(page, "alice");
  const title = unique("Erinnerung Checkout");
  const identifier = await createIssue(page, "ENG", title);
  await page.goto(`/acme/issue/${identifier}`);
  await expect(page.getByLabel("Title")).toHaveValue(title);

  // Shift+H öffnet "Remind me", Auswahl "Tomorrow".
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Shift+H");
  const menu = page.getByRole("menu", { name: "Remind me" });
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitem", { name: "Tomorrow" }).click();
  await expect(page.getByText(/Reminder set for/)).toBeVisible();

  // Zeit vorstellen: die Erinnerung ist jetzt fällig, der periodische Job legt die Notification an.
  sql(
    `UPDATE reminder SET remind_at = now() - interval '1 minute' WHERE issue_id IN (SELECT id FROM issue WHERE identifier = '${identifier}')`,
  );
  await page.goto("/acme/inbox");
  const entry = page.getByRole("option").filter({ hasText: identifier });
  await expect(async () => {
    await page.reload();
    await expect(entry).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 100_000 });
  await expect(entry.getByText("Reminder", { exact: true }).first()).toBeVisible();
});

// F.15: Das Command-Menü findet ein Issue per Titel (Quick results) und öffnet es.
test("F15: command menu finds an issue by title and the i prefix limits the search", async ({
  page,
}) => {
  test.setTimeout(90_000);
  await login(page, "alice");
  const word = unique("Zebrastreifen").replace(" ", "");
  const identifier = await createIssue(page, "ENG", `${word} im Kassenbereich`);
  await page.goto("/acme/inbox");

  const input = page.getByPlaceholder("Type a command or search…");
  const hit = page.getByRole("option").filter({ hasText: identifier });
  // Der Suchindex wird vom Worker nachgeführt: Menü wiederholt öffnen, bis der Treffer erscheint.
  await expect(async () => {
    await page.keyboard.press("Control+k");
    await expect(input).toBeVisible();
    await input.fill(word);
    await expect(hit).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 60_000 });
  await expect(page.getByText("Quick results")).toBeVisible();
  await hit.click();
  await expect(page).toHaveURL(new RegExp(`/acme/issue/${identifier}`));

  // O I öffnet das Menü mit Präfix: nur Issue-Treffer, keine Befehle.
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("o");
  await page.keyboard.press("i");
  await expect(page.getByPlaceholder("Search issues…")).toBeVisible();
  await page.keyboard.type(word);
  await expect(page.getByRole("option").filter({ hasText: identifier })).toBeVisible();
  await expect(page.getByText("Go to Inbox")).toHaveCount(0);
});
