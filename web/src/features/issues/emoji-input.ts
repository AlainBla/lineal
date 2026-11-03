/**
 * Emojis im Editor wie in Linear: `:` öffnet Vorschläge (ab zwei Zeichen), ein fertig getippter
 * Kurzcode `:smile:` wird zu 😄. Eingefügt wird das Zeichen selbst – kein eigener Node, das
 * Markdown/Yjs-Schema bleibt unverändert.
 */
import { Extension, InputRule } from "@tiptap/core";
import { PluginKey } from "@tiptap/pm/state";
import Suggestion, { type SuggestionOptions } from "@tiptap/suggestion";

import { type EmojiEntry, emojiDataSync, loadEmoji, searchEmoji } from "@/lib/emoji";

export interface EmojiInputOptions {
  /** Darstellung der Vorschlagsliste (wie bei Mentions). */
  render: SuggestionOptions<EmojiEntry>["render"];
}

export const emojiSuggestionKey = new PluginKey("lineal-emoji-suggestion");

export const EmojiInput = Extension.create<EmojiInputOptions>({
  name: "emojiInput",
  addOptions() {
    return { render: undefined };
  },
  onCreate() {
    void loadEmoji();
  },
  addInputRules() {
    return [
      new InputRule({
        find: /(?:^|\s):([a-z0-9_+-]+):$/i,
        handler: ({ state, range, match }) => {
          const emoji = emojiDataSync()?.byName.get(match[1]!.toLowerCase());
          if (!emoji) return null;
          // Führendes Leerzeichen der Fundstelle erhalten.
          const from = range.to - (match[1]!.length + 2);
          state.tr.insertText(emoji, from, range.to);
        },
      }),
    ];
  },
  addProseMirrorPlugins() {
    return [
      Suggestion<EmojiEntry>({
        editor: this.editor,
        pluginKey: emojiSuggestionKey,
        char: ":",
        allowSpaces: false,
        items: ({ query }) => {
          const data = emojiDataSync();
          return data && query.length >= 2 ? searchEmoji(data, query, 8) : [];
        },
        command: ({ editor, range, props }) => {
          editor.chain().focus().insertContentAt(range, props.emoji).run();
        },
        render: this.options.render,
      }),
    ];
  },
});
