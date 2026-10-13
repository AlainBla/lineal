import { expect, test } from "@playwright/test";

import { createIssue, login, unique } from "./helpers";

// 1x1-PNG.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

// F.08: Bild in die Beschreibung einfügen (assetUrl gespeichert, signierte /files-URL beim Anzeigen)
// und Datei als Anhang hochladen.
test("F.08: pasted image appears in the description via a signed URL, upload shows in the list", async ({
  page,
}) => {
  await login(page, "bob");
  const id = await createIssue(page, "ENG", unique("Mit Bild"));
  await page.goto(`/acme/issue/${id}`);
  const editor = page.getByRole("textbox", { name: "Description" });
  await editor.click();

  await page.evaluate(async (b64) => {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const file = new File([bytes], "pixel.png", { type: "image/png" });
    const data = new DataTransfer();
    data.items.add(file);
    const el = document.querySelector('[aria-label="Description"][role="textbox"]')!;
    el.dispatchEvent(
      new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }),
    );
  }, PNG.toString("base64"));

  const img = editor.locator("img[alt='pixel.png']");
  await expect(img).toBeVisible();
  await expect(img).toHaveAttribute("src", /^\/files\//);
  await expect.poll(() => img.evaluate((i: HTMLImageElement) => i.naturalWidth)).toBeGreaterThan(0);

  // Gespeichert ist die dauerhafte assetUrl, nie die signierte URL.
  await expect
    .poll(async () =>
      page.evaluate(async (ident) => {
        const r = await fetch("/graphql", {
          method: "POST",
          headers: { "content-type": "application/json", "x-lineal-csrf": "1" },
          body: JSON.stringify({ query: `{ issue(id: "${ident}") { description } }` }),
        });
        return ((await r.json()) as { data: { issue: { description: string | null } } }).data.issue
          .description;
      }, id),
    )
    .toMatch(/!\[pixel\.png\]\(https?:\/\/[^)]*\/assets\/[^)]+\)/);

  // Nach dem Neuladen wird das Bild wieder aufgelöst.
  await page.reload();
  const reloaded = page
    .getByRole("textbox", { name: "Description" })
    .locator("img[alt='pixel.png']");
  await expect(reloaded).toHaveAttribute("src", /^\/files\//);

  // Upload über den Button-Weg (Datei-Input) erscheint in der Anhangsliste.
  await page.getByTestId("attachment-input").setInputFiles({
    name: "notiz.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("hallo"),
  });
  const attachments = page.getByRole("region", { name: "Attachments" });
  await expect(attachments.getByRole("link", { name: "notiz.txt" })).toBeVisible();
});
