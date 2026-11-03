import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import { loadEmoji } from "@/lib/emoji";

import { EmojiInput } from "./emoji-input";

let editor: Editor | null = null;
afterEach(() => editor?.destroy());
beforeAll(async () => {
  await loadEmoji();
});

/** Tippt Zeichen für Zeichen (löst Eingaberegeln aus wie echte Tastatureingabe). */
function type(ed: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = ed.state.selection;
    const handled = ed.view.someProp("handleTextInput", (f) =>
      f(ed.view, from, to, ch, () => ed.state.tr.insertText(ch, from, to)),
    );
    if (!handled) ed.view.dispatch(ed.state.tr.insertText(ch, from, to));
  }
}

describe("EmojiInput", () => {
  it("ersetzt einen fertig getippten Kurzcode durch das Emoji", () => {
    editor = new Editor({ extensions: [StarterKit, EmojiInput], content: "<p></p>" });
    type(editor, "Super :rocket: und :gibtsnicht: ok");
    expect(editor.getText()).toBe("Super 🚀 und :gibtsnicht: ok");
  });

  it("lässt Kurzcodes in Code unverändert", () => {
    editor = new Editor({
      extensions: [StarterKit, EmojiInput],
      content: "<pre><code></code></pre>",
    });
    editor.commands.setTextSelection(1);
    type(editor, ":rocket:");
    expect(editor.getText().trim()).toBe(":rocket:");
  });
});
