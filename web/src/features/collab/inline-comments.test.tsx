import { act, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import * as Y from "yjs";

import "@/app/test-polyfills";

import { RichEditor } from "@/features/issues/rich-editor";

import type { InlineMark } from "./inline-comments";
import { CollabProvider } from "./provider";

// ProseMirror misst Textbereiche; jsdom liefert dafür keine Geometrie.
Range.prototype.getClientRects ??= () =>
  ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const b64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));

function setup(text: string) {
  const provider = new CollabProvider({ url: "ws://x/collab/document/d1" });
  const frag = provider.doc.getXmlFragment("default");
  const para = new Y.XmlElement("paragraph");
  const t = new Y.XmlText(text);
  para.insert(0, [t]);
  frag.insert(0, [para]);
  const anchor = (start: number, end: number): InlineMark => ({
    id: "c1",
    resolved: false,
    anchorStart: b64(Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(t, start))),
    anchorEnd: b64(Y.encodeRelativePosition(Y.createRelativePositionFromTypeIndex(t, end, -1))),
  });
  return { provider, t, anchor };
}

describe("Inline-Kommentare im Editor", () => {
  it("markiert die verankerte Textstelle und lässt sie bei Einfügungen davor mitwandern", async () => {
    const { provider, t, anchor } = setup("Hello brave world");
    render(
      <RichEditor
        value=""
        ariaLabel="Document content"
        collab={{ provider, canWrite: true }}
        inline={{
          marks: [anchor(6, 11)],
          activeId: null,
          onMarkClick: vi.fn(),
          onComment: vi.fn(),
        }}
      />,
    );
    const box = await screen.findByRole("textbox", { name: "Document content" });
    await waitFor(() => expect(box.querySelector("[data-comment-id='c1']")).not.toBeNull());
    expect(box.querySelector("[data-comment-id='c1']")).toHaveTextContent("brave");

    // Ein anderer Nutzer fügt vorne Text ein: die Markierung bleibt an „brave“.
    act(() => t.insert(0, "Oh, "));
    await waitFor(() => expect(box).toHaveTextContent("Oh, Hello brave world"));
    expect(box.querySelector("[data-comment-id='c1']")).toHaveTextContent(/^brave$/);
    provider.destroy();
  });

  it("zeichnet gelöste Threads nicht und nach vollständigem Löschen keine Markierung", async () => {
    const { provider, t, anchor } = setup("Hello brave world");
    const mark = anchor(6, 11);
    const { rerender } = render(
      <RichEditor
        value=""
        ariaLabel="Document content"
        collab={{ provider, canWrite: true }}
        inline={{
          marks: [{ ...mark, resolved: true }],
          activeId: null,
          onMarkClick: vi.fn(),
          onComment: vi.fn(),
        }}
      />,
    );
    const box = await screen.findByRole("textbox", { name: "Document content" });
    await waitFor(() => expect(box).toHaveTextContent("Hello brave world"));
    expect(box.querySelector("[data-comment-id]")).toBeNull();

    rerender(
      <RichEditor
        value=""
        ariaLabel="Document content"
        collab={{ provider, canWrite: true }}
        inline={{ marks: [mark], activeId: "c1", onMarkClick: vi.fn(), onComment: vi.fn() }}
      />,
    );
    await waitFor(() => expect(box.querySelector(".inline-comment-active")).not.toBeNull());

    act(() => t.delete(6, 6));
    await waitFor(() => expect(box).toHaveTextContent("Hello world"));
    expect(box.querySelector("[data-comment-id]")).toBeNull();
    provider.destroy();
  });
});
