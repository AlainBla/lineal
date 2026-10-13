import { expect, test, type Page } from "@playwright/test";

import { login } from "./helpers";

// docs/02 §5.1: Inhalte des privaten Teams DES sieht nur, wer Mitglied ist (Bob). Workspace-Owner
// und -Admins (Alice) sehen das Team als Metadaten (Name/Key), aber keine Issues; Gäste (Carol)
// sehen es gar nicht.
async function gql<T>(page: Page, query: string): Promise<T> {
  return page.evaluate(async (q) => {
    const res = await fetch("/graphql", {
      method: "POST",
      headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
      body: JSON.stringify({ query: q }),
    });
    return (await res.json()) as T;
  }, query);
}

async function visibleTeamKeys(page: Page): Promise<string[]> {
  const body = await gql<{ data: { teams: Array<{ key: string }> } }>(page, "{ teams { key } }");
  return body.data.teams.map((t) => t.key);
}

async function issueIdentifiers(page: Page): Promise<string[]> {
  const body = await gql<{ data: { issues: { nodes: Array<{ identifier: string }> } } }>(
    page,
    "{ issues(first: 250) { nodes { identifier } } }",
  );
  return body.data.issues.nodes.map((i) => i.identifier);
}

test("private team DES: content only for members, metadata for owners, nothing for guests", async ({
  browser,
}) => {
  const bobCtx = await browser.newContext();
  const aliceCtx = await browser.newContext();
  const carolCtx = await browser.newContext();
  const bob = await bobCtx.newPage();
  const alice = await aliceCtx.newPage();
  const carol = await carolCtx.newPage();
  await login(bob, "bob");
  await login(alice, "alice");
  await login(carol, "carol");

  // API-Ebene: Bob sieht DES samt Issues; Alice (Owner) nur die Metadaten; Carol (Gast) nichts.
  expect(await visibleTeamKeys(bob)).toContain("DES");
  expect((await issueIdentifiers(bob)).some((i) => i.startsWith("DES-"))).toBe(true);
  const aliceTeams = await visibleTeamKeys(alice);
  expect(aliceTeams).toContain("ENG");
  expect(aliceTeams).toContain("DES");
  expect((await issueIdentifiers(alice)).some((i) => i.startsWith("DES-"))).toBe(false);
  expect(await visibleTeamKeys(carol)).not.toContain("DES");

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
  await carolCtx.close();
});
