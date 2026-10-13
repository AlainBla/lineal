/**
 * Inline-Kommentare im Dokument-Editor (docs/06 §1.8, F.06): ein ProseMirror-Plugin, das die
 * Anker der Kommentare (Base64 von `Y.encodeRelativePosition`, wie der Server sie speichert) gegen
 * das gemeinsame Yjs-Dokument auflöst und die Textstellen markiert. Weil die Anker relative
 * Positionen sind, wandern die Markierungen bei fremden Einfügungen und Löschungen mit; ist der
 * Text vollständig gelöscht, entfällt die Markierung (der Thread bleibt in der Seitenleiste).
 */
import { Extension } from "@tiptap/core";
import type { EditorState } from "@tiptap/pm/state";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import {
  absolutePositionToRelativePosition,
  relativePositionToAbsolutePosition,
  ySyncPluginKey,
} from "@tiptap/y-tiptap";
import * as Y from "yjs";

export interface InlineMark {
  id: string;
  anchorStart: string;
  anchorEnd: string;
  resolved: boolean;
}

export interface InlineCommentsOptions {
  getMarks: () => InlineMark[];
  getActiveId: () => string | null;
  onMarkClick: (id: string) => void;
}

export const inlineCommentsKey = new PluginKey("lineal-inline-comments");

const b64decode = (s: string): Uint8Array | null => {
  try {
    const bin = atob(s.trim());
    return Uint8Array.from(bin, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
};
const b64encode = (b: Uint8Array): string => {
  let s = "";
  for (const x of b) s += String.fromCharCode(x);
  return btoa(s);
};

interface YState {
  doc: Y.Doc;
  type: Y.XmlFragment;
  binding?: { mapping: Map<Y.AbstractType<unknown>, unknown> };
}

const ystateOf = (state: EditorState): YState | null => {
  const s = ySyncPluginKey.getState(state) as YState | undefined;
  return s?.type && s.binding?.mapping ? s : null;
};

/** Löst einen Anker zu ProseMirror-Positionen auf; `null`, wenn er nicht (mehr) existiert. */
export function resolveAnchor(
  state: EditorState,
  anchorStart: string,
  anchorEnd: string,
): { from: number; to: number } | null {
  const ys = ystateOf(state);
  if (!ys) return null;
  const a = b64decode(anchorStart);
  const b = b64decode(anchorEnd);
  if (!a || !b) return null;
  try {
    const from = relativePositionToAbsolutePosition(
      ys.doc,
      ys.type,
      Y.decodeRelativePosition(a),
      ys.binding!.mapping as never,
    );
    const to = relativePositionToAbsolutePosition(
      ys.doc,
      ys.type,
      Y.decodeRelativePosition(b),
      ys.binding!.mapping as never,
    );
    if (from == null || to == null || from >= to || to > state.doc.content.size) return null;
    return { from, to };
  } catch {
    return null;
  }
}

/** Anker (Base64 relativer Positionen) der aktuellen Auswahl, samt zitiertem Klartext. */
export function selectionAnchor(
  state: EditorState,
): { quotedText: string; anchorStart: string; anchorEnd: string } | null {
  const ys = ystateOf(state);
  const { from, to, empty } = state.selection;
  if (!ys || empty) return null;
  const quotedText = state.doc.textBetween(from, to, " ").trim();
  if (!quotedText) return null;
  const start = absolutePositionToRelativePosition(from, ys.type, ys.binding!.mapping as never);
  const end = absolutePositionToRelativePosition(to, ys.type, ys.binding!.mapping as never);
  return {
    quotedText,
    anchorStart: b64encode(Y.encodeRelativePosition(start)),
    anchorEnd: b64encode(Y.encodeRelativePosition(end)),
  };
}

export const InlineComments = Extension.create<InlineCommentsOptions>({
  name: "inlineComments",
  addOptions() {
    return { getMarks: () => [], getActiveId: () => null, onMarkClick: () => undefined };
  },
  addProseMirrorPlugins() {
    const options = this.options;
    return [
      new Plugin({
        key: inlineCommentsKey,
        props: {
          decorations(state) {
            const decos: Decoration[] = [];
            const active = options.getActiveId();
            for (const m of options.getMarks()) {
              if (m.resolved) continue;
              const r = resolveAnchor(state, m.anchorStart, m.anchorEnd);
              if (!r) continue;
              decos.push(
                Decoration.inline(r.from, r.to, {
                  class: `inline-comment-mark${m.id === active ? " inline-comment-active" : ""}`,
                  "data-comment-id": m.id,
                }),
              );
            }
            return decos.length ? DecorationSet.create(state.doc, decos) : null;
          },
          handleClick(_view, _pos, event) {
            const el = (event.target as HTMLElement | null)?.closest?.("[data-comment-id]");
            const id = el?.getAttribute("data-comment-id");
            if (id) options.onMarkClick(id);
            return false;
          },
        },
      }),
    ];
  },
});
