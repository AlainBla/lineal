import { createHash, randomBytes } from "node:crypto";

import { expect, test } from "@playwright/test";

import { createIssue, login, sql, unique } from "./helpers";
import { API_KEY, callTool } from "./mcp";

// WP32 (docs/09 §5.6): Teilt Bob ein ENG-Issue mit dem Gast Carol (nur Mitglied in WEB), erscheint es
// bei Carol live – ohne Reload –, und der Entzug nimmt es ihr wieder weg.
test("a shared issue appears live for the guest and disappears on unshare", async ({
  browser,
  request,
}) => {
  test.skip(!API_KEY, "run via scripts/e2e.sh (creates the API keys)");

  // Teilen braucht aktivierte Freigaben und einen Team-Owner (Bob ist in ENG nur Mitglied).
  sql(`UPDATE organization SET limits = limits || '{"issueSharingEnabled": true}'::jsonb`);
  // Eigener Key: F22 sperrt Bob zwischendurch, das widerruft dessen übrige API-Keys dauerhaft.
  const key = `lin_api_e2e_f32_${randomBytes(12).toString("hex")}`;
  const hash = createHash("sha256").update(key).digest("hex");
  sql(
    `INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash, scopes, team_ids) SELECT u.organization_id, u.id, 'e2e f32', left('${key}', 12), '${hash}', ARRAY['read','write'], NULL FROM "user" u WHERE u.email = 'bob@acme.example'`,
  );
  sql(
    `UPDATE team_membership SET owner = true WHERE user_id = (SELECT id FROM "user" WHERE email = 'bob@acme.example') AND team_id = (SELECT id FROM team WHERE key = 'ENG')`,
  );

  const bobCtx = await browser.newContext();
  const carolCtx = await browser.newContext();
  const bob = await bobCtx.newPage();
  const carol = await carolCtx.newPage();
  await login(bob, "bob");
  await login(carol, "carol");

  const title = unique("Geteiltes Issue");
  const identifier = await createIssue(bob, "ENG", title);

  // Carol öffnet das Issue: ENG sieht sie nicht, also „nicht gefunden“.
  await carol.goto(`/acme/issue/${identifier}`);
  await expect(carol.getByRole("alert")).toBeVisible();
  await expect(carol.getByText(title)).toHaveCount(0);

  // Bob teilt (MCP `share_issue`, API-Key von Bob): der Sync-Strom stellt es Carol zu.
  const shared = await callTool(request, key, "share_issue", {
    issue: identifier,
    user: "carol@acme.example",
  });
  expect(shared.isError, JSON.stringify(shared.data)).toBe(false);
  await expect(carol.getByText(title).first()).toBeVisible({ timeout: 20_000 });

  // Entzug: das Issue verschwindet bei Carol wieder.
  const unshared = await callTool(request, key, "unshare_issue", {
    issue: identifier,
    user: "carol@acme.example",
  });
  expect(unshared.isError, JSON.stringify(unshared.data)).toBe(false);
  await expect(carol.getByText(title)).toHaveCount(0, { timeout: 20_000 });

  await bobCtx.close();
  await carolCtx.close();
});
