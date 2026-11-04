import { afterEach, describe, expect, it, vi } from "vitest";

import { downloadName, downloadUrl, imageExtension } from "./image-lightbox-store";

afterEach(() => vi.unstubAllGlobals());

describe("downloadName (LIN-21)", () => {
  it("setzt die Endung immer aus dem Inhaltstyp", () => {
    expect(downloadName("Screenshot", "png")).toBe("Screenshot.png");
    expect(downloadName("pixel.png", "png")).toBe("pixel.png");
    expect(downloadName("rechnung.pdf.exe", "png")).toBe("rechnung.pdf.png");
    expect(downloadName("foto.png", "jpg")).toBe("foto.jpg");
  });

  it("entfernt unzulässige, Steuer- und Bidi-Zeichen und fällt auf „image“ zurück", () => {
    expect(downloadName("a/b:c", "gif")).toBe("a_b_c.gif");
    expect(downloadName("evil‮gnp.exe", "png")).toBe("evilgnp.png");
    expect(downloadName("", "webp")).toBe("image.webp");
    expect(downloadName(undefined, "png")).toBe("image.png");
  });

  it("kennt nur Bildtypen", () => {
    expect(imageExtension("image/png")).toBe("png");
    expect(imageExtension("image/jpeg; charset=binary")).toBe("jpg");
    expect(imageExtension("application/x-msdownload")).toBeUndefined();
  });
});

describe("downloadUrl (LIN-21)", () => {
  it("ignoriert fremde und nicht-http(s)-URLs", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    await downloadUrl("javascript:alert(1)", "x");
    await downloadUrl("https://evil.example/x.png", "x");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(click).not.toHaveBeenCalled();
    click.mockRestore();
  });

  it("nutzt bei Nicht-Bildern den direkten Link mit dem Namen des Servers", async () => {
    vi.stubGlobal(
      "fetch",
      async () => new Response(new Blob(["MZ"], { type: "application/x-msdownload" })),
    );
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    await downloadUrl("/files/tok", "bild.png");
    const a = click.mock.contexts.at(-1) as HTMLAnchorElement;
    expect(a.getAttribute("href")).toBe(`${window.location.origin}/files/tok`);
    expect(a.download).toBe("");
    click.mockRestore();
  });
});
