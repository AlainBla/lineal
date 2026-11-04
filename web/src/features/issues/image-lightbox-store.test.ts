import { describe, expect, it } from "vitest";

import { downloadName } from "./image-lightbox-store";

describe("downloadName (LIN-21)", () => {
  it("ergänzt die Endung aus dem Inhaltstyp, wenn sie fehlt", () => {
    expect(downloadName("Screenshot", "image/png")).toBe("Screenshot.png");
    expect(downloadName("foto", "image/jpeg")).toBe("foto.jpg");
  });

  it("lässt vorhandene Endungen und unbekannte Typen unverändert", () => {
    expect(downloadName("pixel.png", "image/png")).toBe("pixel.png");
    expect(downloadName("spec", "application/pdf")).toBe("spec");
  });

  it("ersetzt unzulässige Zeichen und fällt auf „image“ zurück", () => {
    expect(downloadName("a/b:c", "image/gif")).toBe("a_b_c.gif");
    expect(downloadName("", "image/webp")).toBe("image.webp");
    expect(downloadName(undefined, "")).toBe("image");
  });
});
