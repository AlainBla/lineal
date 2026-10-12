import { execFileSync } from "node:child_process";
import path from "node:path";

import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

/** Wie `sql()`, liefert aber das Ergebnis (eine Zelle). */
function sqlValue(statement: string): string {
  const cmd = (
    process.env.LINEAL_E2E_PSQL ?? "docker compose exec -T postgres psql -U lineal -d lineal_e2e"
  ).split(/\s+/);
  return execFileSync(
    cmd[0]!,
    [...cmd.slice(1), "-v", "ON_ERROR_STOP=1", "-t", "-A", "-c", statement],
    {
      cwd: path.resolve(import.meta.dirname, "../.."),
      stdio: "pipe",
    },
  )
    .toString()
    .trim();
}

// WP36 / docs/06 §1.9 Stufe 2: Zwei Browser (Alice, Bob) tippen gleichzeitig im selben Dokument.
test("WP36: two users edit one document concurrently, see each other's cursor, and it persists", async ({
  browser,
}) => {
  const slug = sqlValue("SELECT slug_id FROM document WHERE title = 'Checkout-Spezifikation'");
  expect(slug).toMatch(/^[0-9a-f]{12}$/);

  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");

  for (const page of [alice, bob]) {
    await page.goto(`/acme/document/${slug}`);
    // Echtzeitbetrieb: Status „Live“ statt Autosave-Fallback.
    await expect(page.getByRole("status").filter({ hasText: "Live" })).toBeVisible();
  }
  const aliceEditor = alice.getByRole("textbox", { name: "Document content" });
  const bobEditor = bob.getByRole("textbox", { name: "Document content" });
  await expect(aliceEditor).toContainText("Der bestehende Checkout");
  await expect(bobEditor).toContainText("Der bestehende Checkout");

  const a = unique("Alice");
  const b = unique("Bob");
  // Gleichzeitig an derselben Stelle (Dokumentanfang) tippen.
  await aliceEditor.click();
  await alice.keyboard.press("Control+Home");
  await bobEditor.click();
  await bob.keyboard.press("Control+Home");
  await Promise.all([
    alice.keyboard.type(`${a} `, { delay: 30 }),
    bob.keyboard.type(`${b} `, { delay: 30 }),
  ]);

  // Beide sehen beide Texte.
  for (const editor of [aliceEditor, bobEditor]) {
    await expect(editor).toContainText(a);
    await expect(editor).toContainText(b);
  }

  // Bob sieht Alices Cursor samt Namen und Alice in der Teilnehmerliste.
  await expect(bob.locator(".collaboration-carets__label")).toContainText("Alice");
  await expect(bob.getByRole("list", { name: "Participants" }).getByLabel(/Alice/)).toBeVisible();

  // Persistenz: nach dem Debounce im Dokument gespeichert, nach Neuladen sichtbar.
  await expect
    .poll(
      () =>
        sqlValue(
          `SELECT markdown FROM document_content WHERE owner_type = 'document' AND owner_id = (SELECT id FROM document WHERE slug_id = '${slug}')`,
        ),
      { timeout: 20_000 },
    )
    .toContain(a);
  expect(
    sqlValue(
      `SELECT markdown FROM document_content WHERE owner_type = 'document' AND owner_id = (SELECT id FROM document WHERE slug_id = '${slug}')`,
    ),
  ).toContain(b);
  await alice.reload();
  const reloaded = alice.getByRole("textbox", { name: "Document content" });
  await expect(reloaded).toContainText(a);
  await expect(reloaded).toContainText(b);

  await aliceCtx.close();
  await bobCtx.close();
});

// WP36: Beschreibung eines Issues wird ebenfalls live bearbeitet und über den Issue-Pfad gespeichert.
test("WP36: issue description is edited live by two users and saved on the issue", async ({
  browser,
}) => {
  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");
  const identifier = await createIssue(alice, "ENG", unique("Live-Beschreibung"));

  for (const page of [alice, bob]) {
    await page.goto(`/acme/issue/${identifier}`);
    await expect(page.getByRole("status").filter({ hasText: "Live" })).toBeVisible();
  }
  const a = unique("Alice");
  const b = unique("Bob");
  const aliceEditor = alice.getByRole("textbox", { name: "Description" });
  const bobEditor = bob.getByRole("textbox", { name: "Description" });
  await aliceEditor.click();
  await alice.keyboard.type(`${a} `, { delay: 30 });
  await bobEditor.click();
  await bob.keyboard.type(`${b} `, { delay: 30 });
  for (const editor of [aliceEditor, bobEditor]) {
    await expect(editor).toContainText(a);
    await expect(editor).toContainText(b);
  }
  await expect
    .poll(() => sqlValue(`SELECT description FROM issue WHERE identifier = '${identifier}'`), {
      timeout: 20_000,
    })
    .toContain(b);
  expect(sqlValue(`SELECT description FROM issue WHERE identifier = '${identifier}'`)).toContain(a);
  await alice.reload();
  await expect(alice.getByRole("textbox", { name: "Description" })).toContainText(a);
  await aliceCtx.close();
  await bobCtx.close();
});
