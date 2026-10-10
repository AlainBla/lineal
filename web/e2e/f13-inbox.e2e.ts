import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// docs/08 F13: Eine Erwähnung im Kommentar erzeugt beim zweiten User eine Notification.
test("F13: a mention reaches the other user's inbox and can be marked read", async ({
  browser,
}) => {
  const aliceCtx = await browser.newContext();
  const bobCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bob = await bobCtx.newPage();
  await login(alice, "alice");
  await login(bob, "bob");

  const identifier = await createIssue(alice, "ENG", unique("Checkout hängt"));
  await alice.goto(`/acme/issue/${identifier}`);

  // @-Erwähnung im Kommentar: Vorschlag wählen, Text ergänzen, absenden.
  const editor = alice.locator('[aria-label="Activity"] [role="textbox"]').last();
  await editor.click();
  await alice.keyboard.type("Bitte schauen @Bo");
  await alice.getByRole("option", { name: "Bob" }).click();
  await alice.keyboard.type(" kurz drauf");
  await alice.getByRole("button", { name: "Comment", exact: true }).click();
  await expect(alice.getByLabel("Comment by Alice")).toBeVisible();

  // Bob sieht die Notification (Worker + Sync brauchen einen Moment).
  await bob.goto("/acme/inbox");
  const entry = bob.getByRole("option").filter({ hasText: identifier });
  await expect(async () => {
    await bob.reload();
    await expect(entry).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 40_000 });
  await expect(entry.getByLabel("Unread")).toBeVisible();

  // Öffnen markiert sie als gelesen.
  await entry.click();
  await expect(
    bob.getByRole("option").filter({ hasText: identifier }).getByLabel("Read"),
  ).toBeVisible();

  await aliceCtx.close();
  await bobCtx.close();
});
