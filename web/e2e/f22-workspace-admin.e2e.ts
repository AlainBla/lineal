import { expect, test, type Page } from "@playwright/test";

import { login, sql } from "./helpers";

async function setInvitationsRole(page: Page, label: string) {
  await page.goto("/acme/settings/security");
  await page.getByLabel("Who can invite members?").selectOption({ label });
  await page.getByRole("button", { name: "Save…" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText("Security settings saved").first()).toBeVisible();
}

// WP22: Alice (Owner) ändert Rechte und sperrt Bob; Bobs Browser-Session verliert sofort ihre Wirkung.
test("owner restricts invitations and suspends and unsuspends a member", async ({ browser }) => {
  const aliceCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  const bobCtx = await browser.newContext();
  const bob = await bobCtx.newPage();
  try {
    await login(alice, "alice");
    await login(bob, "bob");

    // Bob (Member) sieht die Security-Seite nicht in der Navigation.
    await bob.goto("/acme/settings/members");
    await expect(bob.getByRole("button", { name: /Invite people/ })).toBeVisible();
    await expect(bob.getByRole("link", { name: "Security" })).toBeHidden();

    // invitationsRole = Admins and owners -> Bob darf nicht mehr einladen.
    await setInvitationsRole(alice, "Admins and owners");
    await bob.reload();
    await expect(bob.getByText(/do not have permission to invite/)).toBeVisible();
    await expect(bob.getByRole("button", { name: /Invite people/ })).toBeHidden();
    // Auch direkt am Server ist das Einladen verboten.
    const res = await bob.evaluate(async () => {
      const r = await fetch("/graphql", {
        method: "POST",
        headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
        body: JSON.stringify({
          query: 'mutation { inviteUsers(input: { emails: ["dora@acme.example"] }) { success } }',
        }),
      });
      return (await r.json()) as { errors?: Array<{ extensions?: { code?: string } }> };
    });
    expect(res.errors?.[0]?.extensions?.code).toBe("forbidden");

    // Zurück auf Standard, damit andere Flows unberührt bleiben.
    await setInvitationsRole(alice, "All members");

    // Alice sperrt Bob (mit Bestätigung).
    await alice.goto("/acme/settings/members");
    await alice.getByRole("button", { name: "Suspend bob@acme.example" }).click();
    await alice.getByRole("dialog").getByRole("button", { name: "Suspend member" }).click();
    await expect(alice.getByLabel("Status of bob@acme.example")).toHaveText("suspended");

    // Bobs bestehende Session ist sofort ungültig.
    const status = await bob.evaluate(async () => (await fetch("/auth/me")).status);
    expect(status).toBe(401);
    await bob.reload();
    await expect(bob).toHaveURL(/\/login/);

    // Gesperrt: auch ein neuer Login scheitert.
    await bob.getByLabel("Email").fill("bob@acme.example");
    await bob.getByLabel("Password").fill("lineal-dev");
    await bob.getByRole("button", { name: "Log in" }).click();
    await expect(bob).toHaveURL(/\/login/);

    // Entsperren; danach geht ein frischer Login wieder.
    await alice.getByRole("button", { name: "Unsuspend bob@acme.example" }).click();
    await expect(alice.getByLabel("Status of bob@acme.example")).toHaveText("active");
    await login(bob, "bob");
    expect(await bob.evaluate(async () => (await fetch("/auth/me")).status)).toBe(200);
  } finally {
    // Nach einem Fehlschlag den Ausgangszustand für die übrigen Flows herstellen.
    sql(`UPDATE "user" SET active = true WHERE email = 'bob@acme.example'`);
    sql(
      `UPDATE organization SET security_settings = security_settings || '{"invitationsRole":"user"}'::jsonb`,
    );
    await aliceCtx.close();
    await bobCtx.close();
  }
});
