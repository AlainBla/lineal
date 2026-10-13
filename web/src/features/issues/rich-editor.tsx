/**
 * Rich-Text-Editor (TipTap) für Beschreibung und Kommentare: Markdown-Ein-/Ausgabe, @-Mentions,
 * `Strg/Cmd+Enter` zum Absenden. Der Server bereinigt den Markdown beim Speichern (markdown::sanitize);
 * Mentions werden als `@Name` serialisiert (WP09 erkennt sie im Text).
 */
import Collaboration from "@tiptap/extension-collaboration";
import CollaborationCaret from "@tiptap/extension-collaboration-caret";
import Image from "@tiptap/extension-image";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Markdown } from "@tiptap/markdown";
import Mention from "@tiptap/extension-mention";
import Placeholder from "@tiptap/extension-placeholder";
import { TableKit } from "@tiptap/extension-table";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import type { EditorView } from "@tiptap/pm/view";
import { useEffect, useRef, useState } from "react";

import type { CollabProvider } from "@/features/collab/provider";
import { cn } from "@/lib/cn";

import { isAssetUrl, isImageFile, resolveAssetUrl } from "./assets";
import type { KnownUser } from "./users";

/**
 * Bild-Node, dessen Markdown-Quelle die dauerhafte `assetUrl` bleibt; im DOM zeigt `<img>` die erst
 * beim Anzeigen aufgelöste, signierte same-origin URL (nie gespeichert, CSP `img-src 'self'`).
 */
const AssetImage = Image.extend({
  addNodeView() {
    return ({ node }) => {
      const img = document.createElement("img");
      const apply = (src: string | null, alt: string | null) => {
        img.alt = alt ?? "";
        img.dataset.src = src ?? "";
        if (src && isAssetUrl(src)) {
          img.removeAttribute("src");
          void resolveAssetUrl(src).then((url) => {
            if (img.dataset.src === src && url) img.src = url;
          });
        } else if (src) {
          img.src = src;
        }
      };
      apply(node.attrs.src as string | null, node.attrs.alt as string | null);
      img.className = "my-1 max-h-96 max-w-full rounded-md";
      return {
        dom: img,
        update: (updated) => {
          if (updated.type !== node.type) return false;
          apply(updated.attrs.src as string | null, updated.attrs.alt as string | null);
          return true;
        },
      };
    };
  },
});

export interface RichEditorProps {
  /** Anfangswert (Markdown); spätere Änderungen erfordern einen neuen `key`. */
  value: string;
  onChange?: (markdown: string) => void;
  /** `Strg/Cmd+Enter`. */
  onSubmit?: () => void;
  onBlur?: () => void;
  placeholder?: string;
  ariaLabel: string;
  /** Personen für `@`-Vorschläge. */
  users?: KnownUser[];
  autoFocus?: boolean;
  /** `false` = nur Anzeige (Kommentartext). */
  editable?: boolean;
  className?: string;
  /**
   * Echtzeit-Mitbearbeitung (Yjs): der Inhalt kommt aus dem gemeinsamen Dokument, `value` wird
   * ignoriert (nie zusätzlich `setContent`, sonst verdoppelt sich der Inhalt). Ohne Schreibrecht
   * ist der Editor schreibgeschützt.
   */
  collab?: { provider: CollabProvider; canWrite: boolean };
  /**
   * Bilder einfügen (Einfügen aus der Zwischenablage, Ziehen in den Editor): lädt die Datei hoch
   * und liefert die dauerhafte `assetUrl`, die als Markdown-Bild gespeichert wird.
   */
  uploadImage?: (file: File) => Promise<string>;
  /** Meldet Fehler beim Hochladen (z. B. als Toast). */
  onUploadError?: (error: unknown) => void;
}

interface Suggestion {
  items: KnownUser[];
  index: number;
  rect: DOMRect | null;
  select: (u: KnownUser) => void;
}

export function RichEditor({
  value,
  onChange,
  onSubmit,
  onBlur,
  placeholder,
  ariaLabel,
  users = [],
  autoFocus,
  editable = true,
  className,
  collab,
  uploadImage,
  onUploadError,
}: RichEditorProps) {
  const usersRef = useRef(users);
  const submitRef = useRef(onSubmit);
  const blurRef = useRef(onBlur);
  const changeRef = useRef(onChange);
  const uploadRef = useRef(uploadImage);
  const uploadErrorRef = useRef(onUploadError);
  useEffect(() => {
    uploadRef.current = uploadImage;
    uploadErrorRef.current = onUploadError;
    usersRef.current = users;
    submitRef.current = onSubmit;
    blurRef.current = onBlur;
    changeRef.current = onChange;
  });
  const [suggestion, setSuggestion] = useState<Suggestion | null>(null);
  const suggestionRef = useRef<Suggestion | null>(null);
  const setSug = (s: Suggestion | null) => {
    suggestionRef.current = s;
    setSuggestion(s);
  };

  const insertImages = async (view: EditorView, files: File[], pos: number) => {
    for (const file of files) {
      try {
        const src = await uploadRef.current!(file);
        const node = view.state.schema.nodes.image?.create({ src, alt: file.name });
        if (!node) return;
        const at = Math.min(pos, view.state.doc.content.size);
        view.dispatch(view.state.tr.insert(at, node));
        pos = at + node.nodeSize;
      } catch (e) {
        uploadErrorRef.current?.(e);
      }
    }
  };

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Kein `protocols`: http/https/mailto sind linkify-Standard; eigene Protokolle würden bei jeder
        // Editor-Instanz neu registriert (Konsolenwarnung "linkifyjs: already initialized", B2.13).
        link: { openOnClick: false },
        // Mit Collaboration übernimmt Yjs Undo/Redo (sonst Konflikt mit dem History-Plugin).
        ...(collab ? { undoRedo: false as const } : {}),
      }),
      Markdown,
      // Tabellen, Aufgabenlisten und Bilder gehören zum Schema des Servers (collab/pm.rs): ohne sie
      // würde y-prosemirror sie beim Öffnen verwerfen und den Inhalt zurückschreiben (B2.02).
      TableKit.configure({ table: { resizable: false } }),
      TaskList,
      TaskItem.configure({ nested: true }),
      AssetImage.configure({ inline: true, allowBase64: false }),
      ...(collab
        ? [
            Collaboration.configure({ document: collab.provider.doc }),
            CollaborationCaret.configure({
              provider: collab.provider,
              user: collab.provider.identity ?? { name: "You", color: "#888888" },
            }),
          ]
        : []),
      Placeholder.configure({ placeholder: placeholder ?? "" }),
      Mention.extend({
        renderMarkdown: (node) => `@${node.attrs?.label ?? node.attrs?.id ?? ""}`,
        // Die Refs werden erst in Editor-Callbacks gelesen, nie beim Rendern.
        // eslint-disable-next-line react-hooks/refs
      }).configure({
        HTMLAttributes: { class: "rounded bg-surface-active px-1 text-primary" },
        renderText: ({ node }) => `@${node.attrs.label ?? node.attrs.id}`,
        suggestion: {
          char: "@",
          items: ({ query }) =>
            usersRef.current
              .filter((u) => u.name.toLowerCase().includes(query.toLowerCase()))
              .slice(0, 6),
          render: () => {
            const apply = (props: {
              items: KnownUser[];
              clientRect?: (() => DOMRect | null) | null;
              command: (p: { id: string; label: string }) => void;
            }) =>
              setSug({
                items: props.items,
                index: 0,
                rect: props.clientRect?.() ?? null,
                select: (u) => props.command({ id: u.id, label: u.name }),
              });
            return {
              onStart: apply,
              onUpdate: apply,
              onKeyDown: ({ event }) => {
                const s = suggestionRef.current;
                if (!s || s.items.length === 0) return false;
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  const step = event.key === "ArrowDown" ? 1 : -1;
                  setSug({ ...s, index: (s.index + step + s.items.length) % s.items.length });
                  return true;
                }
                if (event.key === "Enter" || event.key === "Tab") {
                  const u = s.items[s.index];
                  if (u) s.select(u);
                  return true;
                }
                return false;
              },
              onExit: () => setSug(null),
            };
          },
        },
      }),
    ],
    ...(collab ? {} : { content: value, contentType: "markdown" as const }),
    editable: editable && (collab ? collab.canWrite : true),
    autofocus: autoFocus ? "end" : false,
    editorProps: {
      attributes: {
        "aria-label": ariaLabel,
        role: "textbox",
        "aria-multiline": "true",
        class: "min-h-[4rem] outline-none",
      },
      handlePaste: (view, event) => {
        const files = [...(event.clipboardData?.files ?? [])].filter(isImageFile);
        if (files.length === 0 || !uploadRef.current) return false;
        event.preventDefault();
        void insertImages(view, files, view.state.selection.from);
        return true;
      },
      handleDrop: (view, event) => {
        const files = [...(event.dataTransfer?.files ?? [])].filter(isImageFile);
        if (files.length === 0 || !uploadRef.current) return false;
        event.preventDefault();
        const at = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos;
        void insertImages(view, files, at ?? view.state.selection.from);
        return true;
      },
      handleKeyDown: (_view, event) => {
        if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && submitRef.current) {
          event.preventDefault();
          submitRef.current();
          return true;
        }
        return false;
      },
    },
    onUpdate: ({ editor: ed }) => {
      if (!collab) changeRef.current?.(ed.getMarkdown());
    },
    onBlur: () => blurRef.current?.(),
  });

  return (
    <div
      className={cn(
        "relative text-sm leading-relaxed [&_.ProseMirror_blockquote]:border-l-2 [&_.ProseMirror_blockquote]:pl-3 [&_.ProseMirror_code]:rounded [&_.ProseMirror_code]:bg-surface-active [&_.ProseMirror_code]:px-1 [&_.ProseMirror_h1]:text-lg [&_.ProseMirror_h1]:font-semibold [&_.ProseMirror_h2]:text-base [&_.ProseMirror_h2]:font-semibold [&_.ProseMirror_ol]:list-decimal [&_.ProseMirror_ol]:pl-5 [&_.ProseMirror_p]:my-1 [&_.ProseMirror_p.is-editor-empty:first-child::before]:pointer-events-none [&_.ProseMirror_p.is-editor-empty:first-child::before]:float-left [&_.ProseMirror_p.is-editor-empty:first-child::before]:h-0 [&_.ProseMirror_p.is-editor-empty:first-child::before]:text-subtle-foreground [&_.ProseMirror_p.is-editor-empty:first-child::before]:content-[attr(data-placeholder)] [&_.ProseMirror_pre]:rounded-md [&_.ProseMirror_pre]:bg-surface [&_.ProseMirror_pre]:p-2 [&_.ProseMirror_ul]:list-disc [&_.ProseMirror_ul]:pl-5",
        className,
      )}
    >
      <EditorContent editor={editor} />
      {suggestion && suggestion.items.length > 0 && (
        <ul
          role="listbox"
          aria-label="Mention suggestions"
          className="fixed z-[70] min-w-40 rounded-md border border-border-strong bg-popover p-1 shadow-popover"
          style={{
            left: suggestion.rect?.left ?? 0,
            top: (suggestion.rect?.bottom ?? 0) + 4,
          }}
        >
          {suggestion.items.map((u, i) => (
            <li
              key={u.id}
              role="option"
              aria-selected={i === suggestion.index}
              className={cn(
                "cursor-default rounded px-2 py-1 text-sm",
                i === suggestion.index && "bg-surface-hover",
              )}
              onMouseDown={(e) => {
                e.preventDefault();
                suggestion.select(u);
              }}
            >
              {u.name}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
