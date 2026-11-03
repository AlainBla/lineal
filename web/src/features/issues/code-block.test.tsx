import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import "@/app/test-polyfills";

import { Editor } from "@tiptap/core";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";

import { CodeCopyButton, HighlightedCodeBlock, copyText } from "./code-block";
import { RichEditor } from "./rich-editor";

Range.prototype.getClientRects ??= () =>
  ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const MD = "Text\n\n```js\nconst x = 'a';\nreturn x;\n```\n\nEnde";

describe("Codeblock-Darstellung", () => {
  it("färbt per Fence-Sprache und zeigt einen Kopieren-Knopf, ohne das Dokument zu ändern", async () => {
    const onChange = vi.fn();
    render(<RichEditor value={MD} ariaLabel="Description" onChange={onChange} />);
    const box = await screen.findByRole("textbox", { name: "Description" });
    await waitFor(() => expect(box.querySelector("pre code .hljs-keyword")).not.toBeNull());
    expect(box.querySelector("pre code")?.className).toContain("language-js");
    const btn = box.querySelector("button.code-copy-btn");
    expect(btn).not.toBeNull();
    // Der Knopf ist Dekoration: nicht im Text des Codeblocks.
    expect(box.querySelector("pre")?.textContent).toBe("const x = 'a';\nreturn x;");
  });

  it("kopiert den Text des Blocks", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(<RichEditor value={MD} ariaLabel="Description" editable={false} />);
    const box = await screen.findByRole("textbox", { name: "Description" });
    const btn = await waitFor(() => {
      const b = box.querySelector("button.code-copy-btn");
      expect(b).not.toBeNull();
      return b as HTMLButtonElement;
    });
    btn.click();
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("const x = 'a';\nreturn x;"));
    await waitFor(() => expect(btn.textContent).toBe("Copied"));
  });

  it("copyText meldet Fehlschlag, wenn weder Clipboard noch execCommand gehen", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("no")) },
      configurable: true,
    });
    document.execCommand = vi.fn().mockReturnValue(false);
    expect(await copyText("x")).toBe(false);
  });

  it("behält Node-Name, language-Attribut und Markdown-Roundtrip (schema-sicher)", () => {
    const mk = (highlighted: boolean) =>
      new Editor({
        extensions: [
          StarterKit.configure(highlighted ? { codeBlock: false } : {}),
          Markdown,
          ...(highlighted ? [HighlightedCodeBlock, CodeCopyButton] : []),
        ],
        content: "```rust\nfn main() {}\n```\n\n```\nplain\n```",
        contentType: "markdown",
      });
    const plain = mk(false);
    const hl = mk(true);
    expect(hl.getJSON()).toEqual(plain.getJSON());
    expect(hl.getJSON().content?.[0]).toMatchObject({
      type: "codeBlock",
      attrs: { language: "rust" },
    });
    expect(hl.getMarkdown()).toBe(plain.getMarkdown());
    expect(hl.getMarkdown()).toContain("```rust\nfn main() {}\n```");
    plain.destroy();
    hl.destroy();
  });
});
