import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { AttachmentsSection, formatBytes } from "./attachments";
import { ImageLightbox } from "./image-lightbox";
import { closeLightbox } from "./image-lightbox-store";
import { MarkdownPreview } from "./markdown-preview";
import { closeMarkdownPreview } from "./markdown-preview-store";
import { makeIssue, mockGraphql, renderWithApp } from "./test-fixtures";

const ISSUE = makeIssue({ id: "i1", identifier: "ENG-1" });

const ATT = {
  id: "a1",
  issueId: "i1",
  title: "spec.pdf",
  subtitle: null,
  url: "http://localhost:8080/files/signed",
  sourceType: "upload",
  contentType: "application/pdf",
  size: 2048,
  creatorId: "user-bob",
  createdAt: "2026-10-01T10:00:00Z",
};

function handlers(attachments: unknown[], extra: Record<string, () => unknown> = {}) {
  return {
    IssueExtras: () => ({
      issue: {
        id: "i1",
        subscribed: false,
        children: [],
        relations: {},
        history: [],
        attachments,
      },
    }),
    ...extra,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  closeMarkdownPreview();
  closeLightbox();
});

describe("Anhänge (F.08)", () => {
  it("formatiert Größen", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
  });

  it("listet Anhänge mit same-origin Download-Link und löscht sie", async () => {
    const gql = mockGraphql(
      handlers([ATT], { AttachmentDelete: () => ({ attachmentDelete: { success: true } }) }),
    );
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    const link = await within(region).findByRole("link", { name: "spec.pdf" });
    expect(link).toHaveAttribute("href", "/files/signed");
    await userEvent.click(within(region).getByRole("button", { name: /Remove attachment/ }));
    await waitFor(() => expect(gql.of("AttachmentDelete")).toHaveLength(1));
    expect(gql.of("AttachmentDelete")[0]!.variables).toEqual({ id: "a1" });
  });

  it("lädt eine Datei hoch (Button/Drop) und hängt sie an das Issue", async () => {
    const gql = mockGraphql(
      handlers([], {
        FileUpload: () => ({
          fileUpload: {
            success: true,
            uploadFile: {
              uploadUrl: "http://localhost:8080/uploads/tok",
              assetUrl: "http://localhost:8080/assets/o/a/b",
              headers: [{ key: "content-type", value: "text/plain" }],
            },
          },
        }),
        AttachmentCreate: () => ({ attachmentCreate: { success: true } }),
      }),
    );
    const base = globalThis.fetch as unknown as (...a: unknown[]) => Promise<Response>;
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) =>
      init?.method === "PUT" ? new Response(null, { status: 200 }) : base(input, init),
    );
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    const file = new File(["hi"], "notes.txt", { type: "text/plain" });
    // Drag & Drop auf den Abschnitt.
    fireEvent.drop(region, { dataTransfer: { files: [file], types: ["Files"] } });
    await waitFor(() => expect(gql.of("AttachmentCreate")).toHaveLength(1));
    expect(gql.of("AttachmentCreate")[0]!.variables).toEqual({
      input: { issueId: "i1", url: "http://localhost:8080/assets/o/a/b", title: "notes.txt" },
    });
    // Button-Weg: der versteckte Datei-Input.
    await userEvent.upload(screen.getByTestId("attachment-input"), file);
    await waitFor(() => expect(gql.of("AttachmentCreate")).toHaveLength(2));
  });
  it("öffnet Bild-Anhänge zuerst groß und bietet einen Download an (LIN-21)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a2",
          title: "shot.png",
          contentType: "image/png",
          url: "http://localhost:8080/files/img",
        },
      ]),
    );
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    const base = globalThis.fetch as unknown as (...a: unknown[]) => Promise<Response>;
    const fetched: string[] = [];
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      if (!String(input).endsWith("/files/img")) return base(input, init);
      fetched.push(new URL(String(input)).pathname);
      return new Response("png", { status: 200, headers: { "content-type": "image/png" } });
    });
    await renderWithApp(
      <>
        <AttachmentsSection issue={ISSUE} />
        <ImageLightbox />
      </>,
    );
    const region = await screen.findByRole("region", { name: "Attachments" });
    await userEvent.click(await within(region).findByRole("button", { name: "Download shot.png" }));
    // Über einen Blob, damit der Dateiname nicht der Speicherschlüssel des Servers ist.
    await waitFor(() => expect(click).toHaveBeenCalled());
    expect(fetched).toEqual(["/files/img"]);
    const a = click.mock.contexts.at(-1) as HTMLAnchorElement;
    expect(a.getAttribute("href")).toMatch(/^blob:/);
    expect(a.download).toBe("shot.png");
    click.mockRestore();

    await userEvent.click(await within(region).findByRole("link", { name: "shot.png" }));
    const dialog = await screen.findByRole("dialog", { name: "shot.png" });
    expect(within(dialog).getByRole("img", { name: "shot.png" })).toHaveAttribute(
      "src",
      "/files/img",
    );
  });

  it("blättert mit Pfeilen und Pfeiltasten durch die Bilder der Liste (LIN-56)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a2",
          title: "a.png",
          contentType: "image/png",
          url: "http://localhost:8080/files/imgA",
        },
        { ...ATT, id: "a3", title: "spec.pdf" },
        {
          ...ATT,
          id: "a4",
          title: "b.png",
          contentType: "image/png",
          url: "http://localhost:8080/files/imgB",
        },
      ]),
    );
    await renderWithApp(
      <>
        <AttachmentsSection issue={ISSUE} />
        <ImageLightbox />
      </>,
    );
    const region = await screen.findByRole("region", { name: "Attachments" });
    await userEvent.click(await within(region).findByRole("link", { name: "a.png" }));
    const dialog = await screen.findByRole("dialog");
    // Die PDF zählt nicht mit: die Galerie sind nur die beiden Bilder.
    expect(within(dialog).getByRole("img")).toHaveAttribute("src", "/files/imgA");
    expect(within(dialog).getByText("1 / 2")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Previous file" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Next file" })).toBeEnabled();
    // Vor per Klick.
    await userEvent.click(within(dialog).getByRole("button", { name: "Next file" }));
    expect(within(dialog).getByRole("img")).toHaveAttribute("src", "/files/imgB");
    expect(within(dialog).getByText("2 / 2")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Next file" })).toBeDisabled();
    // Zurück per Pfeiltaste.
    await userEvent.keyboard("{ArrowLeft}");
    expect(within(dialog).getByRole("img")).toHaveAttribute("src", "/files/imgA");
  });

  it("zeigt in der Lightbox keine Pfeile, wenn nur ein Bild vorschaubar ist (LIN-56)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a2",
          title: "only.png",
          contentType: "image/png",
          url: "http://localhost:8080/files/one",
        },
      ]),
    );
    await renderWithApp(
      <>
        <AttachmentsSection issue={ISSUE} />
        <ImageLightbox />
      </>,
    );
    const region = await screen.findByRole("region", { name: "Attachments" });
    await userEvent.click(await within(region).findByRole("link", { name: "only.png" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByRole("button", { name: "Next file" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Previous file" })).toBeNull();
  });

  it("bietet bei Nicht-Bildern keinen eigenen Download-Knopf", async () => {
    mockGraphql(handlers([ATT]));
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    await within(region).findByRole("link", { name: "spec.pdf" });
    expect(within(region).queryByRole("button", { name: "Download spec.pdf" })).toBeNull();
  });

  it("zeigt Markdown-Anhänge in Lineal gerendert an statt herunterzuladen (LIN-43)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a3",
          title: "readme.md",
          contentType: "text/markdown",
          url: "http://localhost:8080/files/md",
        },
      ]),
    );
    const base = globalThis.fetch as unknown as (...a: unknown[]) => Promise<Response>;
    vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
      if (!String(input).endsWith("/files/md")) return base(input, init);
      return new Response("# Überschrift\n\nHallo Welt", {
        status: 200,
        headers: { "content-type": "text/markdown" },
      });
    });
    await renderWithApp(
      <>
        <AttachmentsSection issue={ISSUE} />
        <MarkdownPreview />
      </>,
    );
    const region = await screen.findByRole("region", { name: "Attachments" });
    const link = await within(region).findByRole("link", { name: "readme.md" });
    // Markdown bekommt wie Bilder einen eigenen Download-Knopf.
    expect(within(region).getByRole("button", { name: "Download readme.md" })).toBeInTheDocument();
    // Klick auf den Namen öffnet die gerenderte Vorschau statt des Downloads.
    await userEvent.click(link);
    const dialog = await screen.findByRole("dialog", { name: "readme.md" });
    expect(await within(dialog).findByText("Hallo Welt")).toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "Überschrift" })).toBeInTheDocument();
  });

  it("öffnet HTML-Anhänge mit viewUrl im Tab (Datei-Origin) und bietet Download an (KOL-478)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a4",
          title: "deck.html",
          contentType: "text/html",
          url: "http://localhost:8080/files/deck",
          viewUrl: "https://lineal-files.test/files/deck",
        },
      ]),
    );
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    const link = await within(region).findByRole("link", { name: "deck.html" });
    // Der Namensklick zielt auf die cookielose Datei-Origin (sandboxed Tab), nicht den Download.
    expect(link).toHaveAttribute("href", "https://lineal-files.test/files/deck");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    // Download bleibt verfügbar (App-Origin).
    expect(within(region).getByRole("button", { name: "Download deck.html" })).toBeInTheDocument();
  });

  it("lädt HTML-Anhänge ohne viewUrl weiter herunter (Feature aus)", async () => {
    mockGraphql(
      handlers([
        {
          ...ATT,
          id: "a5",
          title: "page.html",
          contentType: "text/html",
          url: "http://localhost:8080/files/page",
          viewUrl: null,
        },
      ]),
    );
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    const link = await within(region).findByRole("link", { name: "page.html" });
    // Ohne Datei-Origin bleibt es der same-origin Link, kein eigener Download-Knopf.
    expect(link).toHaveAttribute("href", "/files/page");
    expect(within(region).queryByRole("button", { name: "Download page.html" })).toBeNull();
  });
});
