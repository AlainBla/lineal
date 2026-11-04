import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { AttachmentsSection, formatBytes } from "./attachments";
import { ImageLightbox } from "./image-lightbox";
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

afterEach(() => vi.unstubAllGlobals());

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

  it("bietet bei Nicht-Bildern keinen eigenen Download-Knopf", async () => {
    mockGraphql(handlers([ATT]));
    await renderWithApp(<AttachmentsSection issue={ISSUE} />);
    const region = await screen.findByRole("region", { name: "Attachments" });
    await within(region).findByRole("link", { name: "spec.pdf" });
    expect(within(region).queryByRole("button", { name: "Download spec.pdf" })).toBeNull();
  });
});
