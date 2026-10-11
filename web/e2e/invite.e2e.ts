import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import path from "node:path";

import { expect, test } from "@playwright/test";

import { login } from "./helpers";

/**
 * Führt SQL gegen die E2E-Datenbank aus (wie `scripts/e2e.sh`, über den Postgres-Container).
 * `LINEAL_E2E_PSQL` überschreibt den Befehl, z. B. `docker exec -i <container> psql -U lineal -d lineal_e2e`.
 */
function sql(statement: string): void {
  const cmd = (
    process.env.LINEAL_E2E_PSQL ?? "docker compose exec -T postgres psql -U lineal -d lineal_e2e"
  ).split(/\s+/);
  execFileSync(cmd[0]!, [...cmd.slice(1), "-v", "ON_ERROR_STOP=1", "-c", statement], {
    cwd: path.resolve(import.meta.dirname, "../.."),
    stdio: "pipe",
  });
}

// docs/08 F1/F16: Alice lädt eine neue Adresse ein, die Person nimmt über den Link an und landet
// eingeloggt im Workspace. Der Mailer im Dev-Modus versendet nichts und der Token wird nur gehasht
// gespeichert; der Test setzt daher einen bekannten Token (SHA-256 wie `hash_token`) in die DB.
test("F16: an invited person accepts via /invite/<token> and lands in the workspace", async ({
  browser,
}) => {
  const email = `dora-${randomBytes(3).toString("hex")}@acme.example`;
  const token = `e2e-invite-${randomBytes(16).toString("hex")}`;

  const aliceCtx = await browser.newContext();
  const alice = await aliceCtx.newPage();
  await login(alice, "alice");
  await alice.goto("/acme/settings/members");
  await alice.getByRole("button", { name: /Invite people/ }).click();
  const dialog = alice.getByRole("dialog");
  await dialog.getByLabel("Email addresses").fill(email);
  await dialog.getByRole("checkbox", { name: /ENG/ }).check();
  await dialog.getByRole("button", { name: "Send invitations" }).click();
  await expect(dialog).toBeHidden();
  const row = alice.getByRole("listitem").filter({ hasText: email });
  await expect(row).toBeVisible();
  await expect(row.getByLabel(`Status of ${email}`)).toHaveText("pending");

  sql(
    `UPDATE invitation SET token_hash = '${createHash("sha256").update(token).digest("hex")}' WHERE lower(email) = '${email}'`,
  );

  // Zweiter, nicht angemeldeter Browser-Kontext.
  const doraCtx = await browser.newContext();
  const dora = await doraCtx.newPage();
  await dora.goto(`/invite/${token}`);
  // Falsche Adresse: einheitliche Meldung, ohne Details.
  await dora.getByLabel("Email", { exact: true }).fill("mallory@acme.example");
  await dora.getByLabel("Name").fill("Dora Neu");
  await dora.getByLabel("Password", { exact: true }).fill("ein-gutes-passwort");
  await dora.getByLabel("Repeat password").fill("ein-gutes-passwort");
  await dora.getByRole("button", { name: /Create account and join/ }).click();
  await expect(dora.getByRole("alert")).toContainText("no longer valid");

  await dora.goto(`/invite/${token}`);
  await dora.getByLabel("Email", { exact: true }).fill(email);
  await dora.getByLabel("Name").fill("Dora Neu");
  await dora.getByLabel("Password", { exact: true }).fill("ein-gutes-passwort");
  await dora.getByLabel("Repeat password").fill("ein-gutes-passwort");
  await dora.getByRole("button", { name: /Create account and join/ }).click();
  await expect(dora).toHaveURL(/\/acme\/inbox/);
  await expect(dora.getByRole("button", { name: /\(ENG\)/ })).toBeVisible();

  // Eingeloggt und Mitglied von ENG.
  const me = await dora.evaluate(async () => (await fetch("/auth/me")).json());
  expect(me.email).toBe(email);
  expect(me.teams.map((t: { key: string }) => t.key)).toContain("ENG");

  // Der Link ist verbraucht.
  const otherCtx = await browser.newContext();
  const other = await otherCtx.newPage();
  await other.goto(`/invite/${token}`);
  await other.getByLabel("Email", { exact: true }).fill(email);
  await other.getByLabel("Name").fill("Dora Zwei");
  await other.getByLabel("Password", { exact: true }).fill("ein-gutes-passwort");
  await other.getByLabel("Repeat password").fill("ein-gutes-passwort");
  await other.getByRole("button", { name: /Create account and join/ }).click();
  await expect(other.getByRole("alert")).toContainText("no longer valid");

  // Alice sieht die Einladung nicht mehr als offen.
  await alice.reload();
  await expect(alice.getByRole("heading", { name: "Invitations" })).toBeVisible();
  await expect(alice.getByText(email)).toHaveCount(0);

  await aliceCtx.close();
  await doraCtx.close();
  await otherCtx.close();
});
