import { expect, test } from "@playwright/test";

import { login, sql } from "./helpers";

// WP31 / docs/04 §2.5: Projekt im Roadmap per Drag verschieben, Zieldatum ändert sich (und bleibt).
test("WP31: drag a project bar on the roadmap to move its dates", async ({ page }) => {
  // Das Demo-Projekt hat nur ein Zieldatum; ein Start macht aus dem Punkt einen Balken.
  sql(
    "UPDATE project SET start_date = '2026-11-02', start_date_resolution = NULL, " +
      "target_date = '2026-12-18', target_date_resolution = NULL " +
      "WHERE name = 'Marketing-Site Relaunch'",
  );
  await login(page, "bob");
  await page.goto("/acme/roadmap");

  const bar = page.getByRole("button", {
    name: /^Marketing-Site Relaunch, Nov 2, 2026 – Dec 18, 2026/,
  });
  await expect(bar).toBeVisible();
  await bar.scrollIntoViewIfNeeded();
  const box = (await bar.boundingBox())!;

  // Zoom „Month“ = 7 px je Tag: 70 px = 10 Tage nach rechts.
  const saved = page.waitForResponse(
    (r) => r.url().endsWith("/graphql") && (r.request().postData() ?? "").includes("projectUpdate"),
  );
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 35, box.y + box.height / 2, { steps: 5 });
  await page.mouse.move(box.x + box.width / 2 + 70, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const body = (await (await saved).json()) as { errors?: unknown };
  expect(body.errors).toBeUndefined();

  const moved = page.getByRole("button", {
    name: /^Marketing-Site Relaunch, Nov 12, 2026 – Dec 28, 2026/,
  });
  await expect(moved).toBeVisible();

  // Persistiert: nach dem Neuladen steht das neue Zieldatum noch da und die Projektseite zeigt es.
  await page.reload();
  await expect(moved).toBeVisible();
  await moved.click();
  // F.01: die Projektseite zeigt das Zieldatum als bearbeitbares Datumsfeld.
  await expect(page.getByLabel("Target date", { exact: true })).toHaveValue("2026-12-28");

  // Zurück auf den Ausgangszustand (die Flows teilen sich die Datenbank).
  sql(
    "UPDATE project SET start_date = NULL, target_date = '2026-12-18' " +
      "WHERE name = 'Marketing-Site Relaunch'",
  );
});
