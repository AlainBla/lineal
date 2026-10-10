import { expect, test, type Page } from "@playwright/test";

import { login } from "./helpers";

// docs/02: Das private Team DES ist nur für seine Mitglieder sichtbar (Bob ja, Alice nein).
async function visibleTeamKeys(page: Page): Promise<string[]> {
  const body = await page.evaluate(async () => {
    const res = await fetch("/graphql", {
      method: "POST",
      headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
      body: JSON.stringify({ query: "{ teams { key } }" }),
    });
    return (await res.json()) as { data: { teams: Array<{ key: string }> } };
  });
  return body.data.teams.map((t) => t.key);
}

test("private team DES is invisible to Alice and visible to its member Bob", async ({
  browser,
}) => {
  const bobCtx = await browser.newContext();
  const aliceCtx = await browser.newContext();
  const bob = await bobCtx.newPage();
  const alice = await aliceCtx.newPage();
  await login(bob, "bob");
  await login(alice, "alice");

  // API-Ebene
  expect(await visibleTeamKeys(bob)).toContain("DES");
  const aliceTeams = await visibleTeamKeys(alice);
  expect(aliceTeams).toContain("ENG");
  expect(aliceTeams).not.toContain("DES");

  // UI-Ebene: Seitenleiste (die Startseite hat keine Shell) und direkte URL
  await bob.goto("/acme/inbox");
  await alice.goto("/acme/inbox");
  await expect(bob.getByRole("button", { name: /\(DES\)/ })).toBeVisible();
  await expect(alice.getByRole("button", { name: /\(ENG\)/ })).toBeVisible();
  await expect(alice.getByRole("button", { name: /\(DES\)/ })).toHaveCount(0);
  await alice.goto("/acme/team/DES/all");
  await expect(alice.getByRole("alert")).toBeVisible();

  await bobCtx.close();
  await aliceCtx.close();
});
