import { expect, test, type Page } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

async function gql(page: Page, query: string): Promise<void> {
  const res = await page.evaluate(async (q) => {
    const r = await fetch("/graphql", {
      method: "POST",
      headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
      body: JSON.stringify({ query: q }),
    });
    return (await r.json()) as { errors?: unknown };
  }, query);
  expect(res.errors).toBeUndefined();
}

// F.02 / docs/08 F6: ein von einem Nicht-Mitglied angelegtes Issue landet in der Triage und wird
// von Bob mit Accept einsortiert.
test("F6: an outsider's issue lands in WEB triage and Bob accepts it", async ({ browser }) => {
  const bobCtx = await browser.newContext();
  const aliceCtx = await browser.newContext();
  const bob = await bobCtx.newPage();
  const alice = await aliceCtx.newPage();
  try {
    await login(bob, "bob");
    // Triage für WEB einschalten (Teamverwaltung: Bob ist Owner von WEB).
    await bob.goto("/acme/team/WEB/all");
    await gql(
      bob,
      `mutation { teamUpdate(id: "WEB", input: { triageEnabled: true }) { success } }`,
    );

    await login(alice, "alice");
    const title = unique("Button verrutscht");
    const identifier = await createIssue(alice, "WEB", title);

    await bob.goto("/acme/team/WEB/all");
    await expect(bob.getByRole("link", { name: /Triage/ })).toBeVisible();
    await expect(bob.getByLabel(/in triage/)).toContainText("1");

    await bob.getByRole("link", { name: /Triage/ }).click();
    await expect(bob).toHaveURL(/\/team\/WEB\/triage/);
    const list = bob.getByRole("list", { name: "Triage issues" });
    await expect(list.getByText(title)).toBeVisible();
    await expect(bob.getByRole("region", { name: "Triage detail" })).toContainText(identifier);

    await bob
      .getByRole("toolbar", { name: "Triage actions" })
      .getByRole("button", { name: /Accept/ })
      .click();
    const dialog = bob.getByRole("dialog");
    await dialog.getByLabel("Comment").fill("Übernommen");
    await dialog.getByRole("button", { name: "Accept", exact: true }).click();
    await expect(bob.getByText(`${identifier} accepted`)).toBeVisible();

    // Raus aus der Triage-Liste, rein in die Team-Liste (Backlog), Zähler weg.
    await expect(bob.getByText("Triage is empty")).toBeVisible();
    await bob.goto("/acme/team/WEB/backlog");
    await expect(bob.getByRole("list", { name: "Issues" }).getByText(title)).toBeVisible();
  } finally {
    await bobCtx.close();
    await aliceCtx.close();
  }
});
