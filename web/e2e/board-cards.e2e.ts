import path from "node:path";

import { expect, test, type Page } from "@playwright/test";

import { createIssue, login, sql, unique } from "./helpers";

// Board (docs/08 F5): gleich hohe Karten mit Hover-Details, Drag ohne Rückflug, Hidden columns.
// Screenshots (gitignored) landen im Hauptrepo unter var/board-check/.
const SHOTS =
  process.env.LINEAL_E2E_SHOTS ?? path.resolve(import.meta.dirname, "../../var/board-check");
const PHASE = process.env.LINEAL_E2E_PHASE ?? "nachher";
const shot = (page: Page, name: string) =>
  page.screenshot({ path: path.join(SHOTS, `${PHASE}-${name}.png`) });

async function setStatus(page: Page, identifier: string, name: string): Promise<void> {
  await page.goto(`/acme/issue/${identifier}`);
  await expect(page.locator('[data-property="Status"]')).toBeVisible();
  await page.keyboard.press("s");
  const picker = page.getByRole("dialog");
  await picker.getByPlaceholder("Change status…").fill(name);
  await picker
    .getByRole("option", { name: new RegExp(`${name}$`) })
    .first()
    .click();
  await expect(picker).toBeHidden();
  await expect(page.locator('[data-property="Status"]')).toContainText(name);
}

async function openBoard(page: Page): Promise<void> {
  await page.addInitScript(() => localStorage.setItem("lineal-issues-layout:ENG:all", "board"));
  await page.goto("/acme/team/ENG/all");
  await expect(page.getByRole("region", { name: "Todo" })).toBeVisible();
}

const card = (page: Page, title: string) =>
  page.locator("a[data-issue-id]").filter({ hasText: title });

/** Zieht per Maus in kleinen Schritten; `release: false` lässt die Maus unten (für Abbruch). */
async function dragTo(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 10, from.y + 4, { steps: 3 });
  await page.mouse.move(to.x, to.y, { steps: 20 });
  await page.waitForTimeout(150);
}

interface Sample {
  t: number;
  /** Spalte, in der ein Element mit dem Kartentitel liegt; "overlay" = außerhalb einer Spalte. */
  where: string;
  cx: number;
  opacity: number;
}

/** Startet einen rAF-Sampler: je Frame Position aller Titel-Elemente der Karte. */
async function startSampler(page: Page, title: string): Promise<void> {
  await page.evaluate((t) => {
    const w = window as unknown as { __samples: Sample[]; __stop: boolean };
    w.__samples = [];
    w.__stop = false;
    const t0 = performance.now();
    const tick = () => {
      for (const el of document.querySelectorAll('[data-testid="card-title"]')) {
        if (el.textContent !== t) continue;
        const r = el.getBoundingClientRect();
        const col = el.closest("section[data-column]");
        const cs = getComputedStyle(el.closest("a") ?? el.parentElement!);
        w.__samples.push({
          t: performance.now() - t0,
          where: col?.getAttribute("aria-label") ?? "overlay",
          cx: r.x + r.width / 2,
          opacity: Number(cs.opacity),
        });
      }
      if (!w.__stop) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, title);
}

async function stopSampler(page: Page): Promise<Sample[]> {
  return page.evaluate(() => {
    const w = window as unknown as { __samples: Sample[]; __stop: boolean };
    w.__stop = true;
    return w.__samples;
  });
}

test("board: equal card heights, one-line titles and hover details", async ({ page }) => {
  await login(page, "alice");
  const short = unique("Kurz");
  const long = unique(
    "Ein sehr langer Titel der niemals in eine einzige Zeile der Karte passt und deshalb abgekürzt werden muss, weil sonst die Karte höher wird als alle anderen Karten im Board",
  );
  const labelled = unique("Viele Labels");
  const ids: string[] = [];
  for (const t of [short, long, labelled]) {
    const id = await createIssue(page, "ENG", t);
    await setStatus(page, id, "Todo");
    ids.push(id);
  }
  const names = [
    "Sehr-langer-Label-Name-eins",
    "Zweites-langes-Label",
    "Drittes",
    "Viertes",
    "Fünftes",
  ];
  sql(
    `INSERT INTO issue_label (organization_id, name, color) SELECT organization_id, n, '#4f8cff' FROM issue, unnest(ARRAY[${names.map((n) => `'${n}-${ids[2]}'`).join(",")}]) AS n WHERE identifier = '${ids[2]}'`,
  );
  sql(
    `INSERT INTO issue_to_label (organization_id, issue_id, label_id) SELECT i.organization_id, i.id, l.id FROM issue i JOIN issue_label l ON l.organization_id = i.organization_id AND l.name LIKE '%-${ids[2]}' WHERE i.identifier = '${ids[2]}'`,
  );

  await openBoard(page);
  await expect(card(page, labelled)).toBeVisible();
  await shot(page, "karten");

  const heights: number[] = [];
  for (const t of [short, long, labelled]) {
    const box = await card(page, t).boundingBox();
    heights.push(Math.round(box!.height));
  }
  expect(new Set(heights).size, `Kartenhöhen ${heights.join(", ")}`).toBe(1);
  // Titel einzeilig: Textbreite übersteigt die Box, sichtbar aber abgeschnitten.
  const titleBox = await card(page, long)
    .getByTestId("card-title")
    .evaluate((el) => ({
      clipped: el.scrollWidth > el.clientWidth,
      h: el.getBoundingClientRect().height,
    }));
  expect(titleBox.clipped).toBe(true);
  expect(titleBox.h).toBeLessThan(24);
  // Label-Zeile bleibt einzeilig; Überlauf als „+N“.
  await expect(card(page, labelled)).toContainText("+3");

  // Hover zeigt den vollen Titel und alle Labels.
  await card(page, long).hover();
  // Radix lässt geschlossene Tooltips kurz ausblenden: nur offene zählen.
  const hover = page.locator('[data-testid="card-hover"]:not([data-state="closed"])');
  await expect(hover).toContainText(long);
  await expect(hover).toContainText("Todo");
  await shot(page, "hover-langer-titel");
  await page.mouse.move(5, 300, { steps: 8 });
  await expect(hover).toBeHidden();
  await card(page, labelled).hover();
  for (const n of names) await expect(hover).toContainText(n);
  await shot(page, "hover-labels");
});

test("board: dropping Todo to In Progress does not fly back; cancel does", async ({ page }) => {
  await login(page, "alice");
  const title = unique("Drag");
  const id = await createIssue(page, "ENG", title);
  await setStatus(page, id, "Todo");
  await openBoard(page);
  const todo = page.getByRole("region", { name: "Todo" });
  const progress = page.getByRole("region", { name: "In Progress" });
  await expect(card(page, title)).toBeVisible();

  const from = (await card(page, title).boundingBox())!;
  const target = (await progress.boundingBox())!;
  const todoBox = (await todo.boundingBox())!;
  const start = { x: from.x + from.width / 2 - 20, y: from.y + from.height / 2 };
  const end = { x: target.x + target.width / 2, y: target.y + 80 };

  await dragTo(page, start, end);
  await shot(page, "drag-mitte");
  await startSampler(page, title);
  await page.mouse.up();
  // Bildfolge direkt nach dem Drop (Sichtprüfung der Animation).
  for (let i = 0; i < 6; i++) {
    await shot(page, `drag-frame-${i}`);
    await page.waitForTimeout(40);
  }
  await page.waitForTimeout(700);
  const samples = await stopSampler(page);
  await shot(page, "drag-nach-drop");

  // Karte steht in der Zielspalte und serverseitig im neuen Status.
  await expect(progress.getByText(title)).toBeVisible();
  await expect(todo.getByText(title)).toHaveCount(0);

  // Nach dem Drop darf nichts zur Ausgangsspalte zurückfliegen (kein Overlay links der Zielspalte).
  const overlay = samples.filter((s) => s.t > 0 && s.where === "overlay");
  const back = overlay.filter((s) => s.cx < target.x);
  expect(back, `Overlay flog zurück: ${JSON.stringify(back.slice(0, 5))}`).toHaveLength(0);
  // Kein Frame mit dem Original in der Ausgangsspalte länger als wenige Frames.
  const inOrigin = samples.filter((s) => s.where === "Todo" && s.cx < todoBox.x + todoBox.width);
  expect(inOrigin.length, `Frames in Todo nach Drop: ${inOrigin.length}`).toBeLessThanOrEqual(3);
  await page.reload();
  await expect(page.getByRole("region", { name: "In Progress" }).getByText(title)).toBeVisible();

  // Abbruch mit Escape: Karte bleibt in der Ausgangsspalte, darf zurückfliegen.
  const back2 = (await card(page, title).boundingBox())!;
  const todoCol = (await page.getByRole("region", { name: "Todo" }).boundingBox())!;
  await dragTo(
    page,
    { x: back2.x + back2.width / 2 - 20, y: back2.y + back2.height / 2 },
    { x: todoCol.x + todoCol.width / 2, y: todoCol.y + 80 },
  );
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);
  await page.mouse.up();
  await expect(page.getByRole("region", { name: "In Progress" }).getByText(title)).toBeVisible();
});

test("board: reorder inside a column keeps the card in place", async ({ page }) => {
  await login(page, "alice");
  // Hohes Fenster: beide Karten samt Drop-Ziel müssen ohne Scrollen sichtbar sein.
  await page.setViewportSize({ width: 1280, height: 1400 });
  const a = unique("Sort-A");
  const b = unique("Sort-B");
  for (const t of [a, b]) {
    const id = await createIssue(page, "ENG", t);
    await setStatus(page, id, "In Progress");
  }
  await openBoard(page);
  // Umsortieren wirkt nur bei manueller Sortierung (sonst gilt Priorität/Zeit).
  const manual = async () => {
    await page.getByRole("button", { name: "Display" }).click();
    await page.getByLabel("Ordering", { exact: true }).selectOption("manual");
    await page.keyboard.press("Escape");
  };
  await manual();
  const col = page.getByRole("region", { name: "In Progress" });
  const order = async () =>
    (await col.getByTestId("card-title").allTextContents()).filter((t) => t === a || t === b);
  const before = await order();
  expect(before).toHaveLength(2);
  const first = (await card(page, before[0]!).boundingBox())!;
  const second = (await card(page, before[1]!).boundingBox())!;
  await dragTo(
    page,
    { x: first.x + 100, y: first.y + first.height / 2 },
    { x: second.x + 100, y: second.y + second.height - 4 },
  );
  await shot(page, "umsortieren-mitte");

  await startSampler(page, before[0]!);
  await page.mouse.up();
  await page.waitForTimeout(700);
  const samples = await stopSampler(page);
  await shot(page, "umsortieren-nach-drop");
  expect(await order()).toEqual([before[1], before[0]]);
  // Kein Frame mit einer zweiten Kopie der Karte (Overlay) nach dem Drop.
  expect(samples.filter((s) => s.t > 60 && s.where === "overlay")).toHaveLength(0);
  // Die Reihenfolge ist serverseitig gespeichert (Anzeigeoptionen selbst nicht).
  await page.reload();
  await expect(col.getByTestId("card-title").filter({ hasText: a })).toBeVisible();
  await manual();
  expect(await order()).toEqual([before[1], before[0]]);
});

test("board: Hidden columns lists every hidden column with its count", async ({ page }) => {
  await login(page, "alice");
  const t1 = unique("Verworfen");
  const t2 = unique("Doppelt");
  const id1 = await createIssue(page, "ENG", t1);
  await setStatus(page, id1, "Canceled");
  const id2 = await createIssue(page, "ENG", t2);
  // Der Status-Picker bietet Duplicate nicht an (nur über Relation); daher direkt in der DB.
  sql(
    `UPDATE issue SET state_id = (SELECT w.id FROM workflow_state w JOIN team t ON t.id = w.team_id WHERE t.key = 'ENG' AND w.type = 'duplicate') WHERE identifier = '${id2}'`,
  );
  await openBoard(page);
  const hidden = page.getByRole("complementary", { name: "Hidden columns" });
  await shot(page, "hidden-columns");
  await expect(hidden).toBeVisible();
  const canceled = hidden.getByRole("button", { name: "Show Canceled column" });
  const duplicate = hidden.getByRole("button", { name: "Show Duplicate column" });
  await expect(canceled).toBeVisible();
  await expect(duplicate).toBeVisible();
  await expect(canceled).toContainText(/\d+/);
  await expect(duplicate).toContainText(/\d+/);
  // Standardmäßig bleiben die Spalten zu; Aufklappen zeigt je Spalte ihre Issues.
  await expect(page.getByRole("region", { name: "Canceled" })).toHaveCount(0);
  await canceled.click();
  await duplicate.click();
  await expect(page.getByRole("region", { name: "Canceled" }).getByText(t1)).toBeVisible();
  await expect(page.getByRole("region", { name: "Duplicate" }).getByText(t2)).toBeVisible();
  await shot(page, "hidden-columns-offen");
  await hidden.getByRole("button", { name: "Hide Canceled column" }).click();
  await expect(page.getByRole("region", { name: "Canceled" })).toHaveCount(0);
});
