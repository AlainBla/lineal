/**
 * Codeblöcke wie in Linear: Syntaxfarben nach Fence-Sprache und ein „Kopieren“-Knopf je Block.
 *
 * Schema-sicher: `CodeBlockLowlight` behält den Node-Namen `codeBlock` samt Attribut `language`
 * und färbt nur über ProseMirror-Dekorationen; der Knopf ist ein Widget-Decoration. Dokument,
 * Markdown und Yjs-Inhalt bleiben unverändert (Server-Schema in collab/pm.rs).
 */
import { Extension } from "@tiptap/core";
import CodeBlockLowlight from "@tiptap/extension-code-block-lowlight";
import type { Node as PmNode } from "@tiptap/pm/model";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import { common, createLowlight } from "lowlight";

const lowlight = createLowlight(common);

/** Codeblock mit Syntax-Highlighting (gleicher Node-Name `codeBlock` wie in StarterKit). */
export const HighlightedCodeBlock = CodeBlockLowlight.configure({ lowlight });

export const codeCopyKey = new PluginKey("lineal-code-copy");

/** Text in die Zwischenablage; mit Fallback für Umgebungen ohne Clipboard-API. */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* Fallback unten */
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}

function copyWidget(text: string): HTMLElement {
  const anchor = document.createElement("div");
  anchor.className = "code-copy-anchor";
  anchor.contentEditable = "false";
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "code-copy-btn";
  btn.textContent = "Copy";
  btn.setAttribute("aria-label", "Copy code");
  // Fokus/Selektion im Editor nicht verlieren.
  btn.addEventListener("mousedown", (e) => e.preventDefault());
  btn.addEventListener("click", (e) => {
    e.preventDefault();
    void copyText(text).then((ok) => {
      btn.textContent = ok ? "Copied" : "Failed";
      setTimeout(() => {
        btn.textContent = "Copy";
      }, 1500);
    });
  });
  anchor.appendChild(btn);
  return anchor;
}

export function codeCopyDecorations(doc: PmNode): DecorationSet {
  const decos: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (node.type.name !== "codeBlock") return true;
    const text = node.textContent;
    if (text) {
      decos.push(
        Decoration.widget(pos, () => copyWidget(text), {
          side: -1,
          key: `code-copy:${text}`,
          ignoreSelection: true,
          stopEvent: () => true,
        }),
      );
    }
    return false;
  });
  return DecorationSet.create(doc, decos);
}

export const CodeCopyButton = Extension.create({
  name: "codeCopyButton",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: codeCopyKey,
        state: {
          init: (_c, state) => codeCopyDecorations(state.doc),
          apply: (tr, old, _o, state) => (tr.docChanged ? codeCopyDecorations(state.doc) : old),
        },
        props: {
          decorations(state) {
            return codeCopyKey.getState(state) as DecorationSet;
          },
        },
      }),
    ];
  },
});
