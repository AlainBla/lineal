import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";
import { mockGraphql, newClient, renderWithApp } from "@/features/issues/test-fixtures";
import { shortcuts } from "@/lib/shortcuts";

import { ResourcesSection } from "./resources";

const openLightbox = vi.fn();
const openMarkdownPreview = vi.fn();

vi.mock("@/features/issues/image-lightbox-store", () => ({
  openLightbox: (...a: unknown[]) => openLightbox(...a),
  downloadUrl: vi.fn(),
}));
vi.mock("@/features/issues/markdown-preview-store", () => ({
  openMarkdownPreview: (...a: unknown[]) => openMarkdownPreview(...a),
  isMarkdown: (_ct: string | null, name: string) => name.endsWith(".md"),
}));

const FILES = [
  {
    id: "f-img",
    title: "mockup.png",
    contentType: "image/png",
    size: 4096,
    downloadUrl: "/files/img-signed",
    createdAt: "2026-01-01T00:00:00Z",
  },
  {
    id: "f-pdf",
    title: "plan.pdf",
    contentType: "application/pdf",
    size: 2048,
    downloadUrl: "/files/pdf-signed",
    createdAt: "2026-01-01T00:00:00Z",
  },
];

const base = {
  PlanningResources: () => ({ resources: { documents: [], links: [], files: FILES } }),
};

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.clearAllMocks();
  shortcuts.reset();
});

describe("Projekt-Anhänge (LIN-45)", () => {
  it("Bild-Datei öffnet die Lightbox statt herunterzuladen; PDF bleibt Download", async () => {
    mockGraphql(base);
    await renderWithApp(
      <ResourcesSection ws="acme" projectId="p-1" />,
      newClient(),
      "/acme/project/P-ENG-1/resources",
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("link", { name: "mockup.png" }));
    expect(openLightbox).toHaveBeenCalledWith(
      expect.objectContaining({ src: "/files/img-signed", filename: "mockup.png" }),
    );
    // PDF ist ein echter Download-Link, keine Vorschau.
    const pdf = screen.getByRole("link", { name: "plan.pdf" });
    expect(pdf).toHaveAttribute("download", "plan.pdf");
    await user.click(pdf);
    expect(openMarkdownPreview).not.toHaveBeenCalled();
  });
});
