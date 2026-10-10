import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { RichEditor } from "./rich-editor";

// ProseMirror misst Textbereiche; jsdom liefert dafür keine Geometrie.
Range.prototype.getClientRects ??= () =>
  ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

describe("RichEditor (TipTap)", () => {
  it("rendert Markdown als formatierten Text", async () => {
    render(<RichEditor value={"Hallo **Welt**"} ariaLabel="Description" onChange={vi.fn()} />);
    const box = await screen.findByRole("textbox", { name: "Description" });
    await waitFor(() => expect(box.querySelector("strong")).toHaveTextContent("Welt"));
    expect(box).toHaveAttribute("contenteditable", "true");
  });

  it("ist ohne editable nur zur Anzeige", async () => {
    render(<RichEditor value="Nur lesen" ariaLabel="Comment text" editable={false} />);
    const box = await screen.findByRole("textbox", { name: "Comment text" });
    expect(box).toHaveAttribute("contenteditable", "false");
    expect(box).toHaveTextContent("Nur lesen");
  });
});
