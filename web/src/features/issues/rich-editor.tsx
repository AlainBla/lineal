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
import { QueryClientContext } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import type { EditorView } from "@tiptap/pm/view";
import { useContext, useEffect, useRef, useState } from "react";

import {
  InlineComments,
  type InlineMark,
  inlineCommentsKey,
  selectionAnchor,
} from "@/features/collab/inline-comments";
import type { CollabProvider } from "@/features/collab/provider";
import { cn } from "@/lib/cn";
import type { EmojiEntry } from "@/lib/emoji";

import { isAssetUrl, isImageFile, resolveAssetUrl } from "./assets";
import { CodeCopyButton, HighlightedCodeBlock } from "./code-block";
import { EmojiInput } from "./emoji-input";
import { issueRefStore } from "./issue-ref-store";
import { downloadUrl, openLightbox } from "./image-lightbox-store";
import { IssueRefs, issueRefsKey } from "./issue-refs";
import { type LinkNavigator, linkTarget, openLink } from "./open-link";
import type { KnownUser } from "./users";

const ICON_ATTRS =
  'xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"';
/** Lucide `Maximize2` und `Download` als statisches SVG (Node-Views rendern ohne React). */
const ICON_EXPAND = `<svg ${ICON_ATTRS}><polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/></svg>`;
const ICON_DOWNLOAD = `<svg ${ICON_ATTRS}><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/></svg>`;

function imageAction(label: string, icon: string, run: () => void): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "image-action-btn";
  btn.innerHTML = icon;
  btn.title = label;
  btn.setAttribute("aria-label", label);
  // Fokus und Auswahl im Editor nicht verändern.
  btn.addEventListener("mousedown", (e) => e.preventDefault());
  btn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    run();
  });
  return btn;
}

/**
 * Bild-Node, dessen Markdown-Quelle die dauerhafte `assetUrl` bleibt; im DOM zeigt `<img>` die erst
 * beim Anzeigen aufgelöste, signierte same-origin URL (nie gespeichert, CSP `img-src 'self'`).
 * Oben rechts bieten zwei Icons „vergrößern“ (Lightbox) und „herunterladen“ an (LIN-21).
 */
const AssetImage = Image.extend({
  addNodeView() {
    return ({ node }) => {
      const dom = document.createElement("span");
      dom.className = "image-frame";
      const img = document.createElement("img");
      let attrs = { src: node.attrs.src as string | null, alt: node.attrs.alt as string | null };
      const apply = (src: string | null, alt: string | null) => {
        attrs = { src, alt };
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
      apply(attrs.src, attrs.alt);
      img.className = "max-h-96 max-w-full rounded-md";
      // Beim Klick neu auflösen: die signierte URL im `<img>` kann inzwischen abgelaufen sein.
      const viewUrl = async () => {
        const { src } = attrs;
        if (!src) return null;
        return isAssetUrl(src) ? resolveAssetUrl(src) : src;
      };
      const actions = document.createElement("span");
      actions.className = "image-actions";
      actions.contentEditable = "false";
      actions.append(
        imageAction("Enlarge image", ICON_EXPAND, () => {
          void viewUrl().then((url) => {
            if (url)
              openLightbox({ src: url, alt: attrs.alt ?? "", filename: attrs.alt ?? undefined });
          });
        }),
        imageAction("Download image", ICON_DOWNLOAD, () => {
          void viewUrl().then((url) => {
            if (url) void downloadUrl(url, attrs.alt ?? undefined);
          });
        }),
      );
      dom.append(img, actions);
      return {
        dom,
        update: (updated) => {
          if (updated.type !== node.type) return false;
          apply(updated.attrs.src as string | null, updated.attrs.alt as string | null);
          return true;
        },
        stopEvent: (event) => event.target instanceof Node && actions.contains(event.target),
        ignoreMutation: (m) => m.type !== "selection",
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
   * Inline-Kommentare (nur mit `collab`, F.06): Markierungen im Text, Klick darauf wählt den Thread,
   * und eine Auswahl bietet „Comment“ an (liefert Zitat und Anker).
   */
  inline?: {
    marks: InlineMark[];
    activeId: string | null;
    onMarkClick: (id: string) => void;
    onComment: (a: { quotedText: string; anchorStart: string; anchorEnd: string }) => void;
  };
  /**
   * Bilder einfügen (Einfügen aus der Zwischenablage, Ziehen in den Editor): lädt die Datei hoch
   * und liefert die dauerhafte `assetUrl`, die als Markdown-Bild gespeichert wird.
   */
  uploadImage?: (file: File) => Promise<string>;
  /** Meldet Fehler beim Hochladen (z. B. als Toast). */
  onUploadError?: (error: unknown) => void;
}

/** Eintrag der Vorschlagsliste (Personen für `@`, Emojis für `:`). */
interface SuggestItem {
  id: string;
  label: string;
}

interface Suggestion {
  items: SuggestItem[];
  index: number;
  rect: DOMRect | null;
  select: (item: SuggestItem) => void;
  /** Zugänglicher Name der Liste. */
  label: string;
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
  inline,
  uploadImage,
  onUploadError,
}: RichEditorProps) {
  const router = useRouter({ warn: false });
  const routerRef = useRef<LinkNavigator | undefined>(router);
  useEffect(() => {
    routerRef.current = router;
  }, [router]);
  // Ticket-Chips brauchen den Query-Cache; ohne Provider (isolierte Tests) bleibt der Text schlicht.
  const queryClient = useContext(QueryClientContext);
  const refStore = queryClient ? issueRefStore(queryClient) : null;
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
  const inlineRef = useRef(inline);
  useEffect(() => {
    inlineRef.current = inline;
  });
  const [selBox, setSelBox] = useState<{ left: number; top: number } | null>(null);
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

  // Tastatur in der Vorschlagsliste (Mentions und Emojis teilen sich die Liste).
  const suggestKeyDown = ({ event }: { event: KeyboardEvent }) => {
    const s = suggestionRef.current;
    if (!s || s.items.length === 0) return false;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const step = event.key === "ArrowDown" ? 1 : -1;
      setSug({ ...s, index: (s.index + step + s.items.length) % s.items.length });
      return true;
    }
    if (event.key === "Enter" || event.key === "Tab") {
      const item = s.items[s.index];
      if (item) s.select(item);
      return true;
    }
    return false;
  };
  const onExit = () => setSug(null);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Ersetzt durch HighlightedCodeBlock (gleicher Node-Name `codeBlock`, nur Dekorationen).
        codeBlock: false,
        // Kein `protocols`: http/https/mailto sind linkify-Standard; eigene Protokolle würden bei jeder
        // Editor-Instanz neu registriert (Konsolenwarnung "linkifyjs: already initialized", B2.13).
        link: { openOnClick: false, HTMLAttributes: { rel: "noopener noreferrer", target: null } },
        // Mit Collaboration übernimmt Yjs Undo/Redo (sonst Konflikt mit dem History-Plugin).
        ...(collab ? { undoRedo: false as const } : {}),
      }),
      Markdown,
      HighlightedCodeBlock,
      CodeCopyButton,
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
            // Die Refs werden erst in Editor-Callbacks gelesen, nie beim Rendern.
            // eslint-disable-next-line react-hooks/refs
            InlineComments.configure({
              getMarks: () => inlineRef.current?.marks ?? [],
              getActiveId: () => inlineRef.current?.activeId ?? null,
              onMarkClick: (id) => inlineRef.current?.onMarkClick(id),
            }),
          ]
        : []),
      Placeholder.configure({ placeholder: placeholder ?? "" }),
      ...(refStore
        ? [
            IssueRefs.configure({
              getKeys: () => refStore.keys(),
              lookup: (identifier) => refStore.lookup(identifier),
              hrefFor: (identifier) => {
                const ws = window.location.pathname.split("/")[1] ?? "";
                return `/${ws}/issue/${identifier}`;
              },
              lookupProject: (identifier) => refStore.lookupProject(identifier),
              projectHrefFor: (identifier) => {
                const ws = window.location.pathname.split("/")[1] ?? "";
                return `/${ws}/project/${identifier}/overview`;
              },
            }),
          ]
        : []),
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
                items: props.items.map((u) => ({ id: u.id, label: u.name })),
                index: 0,
                rect: props.clientRect?.() ?? null,
                select: (u) => props.command({ id: u.id, label: u.label }),
                label: "Mention suggestions",
              });
            return { onStart: apply, onUpdate: apply, onKeyDown: suggestKeyDown, onExit };
          },
        },
      }),
      EmojiInput.configure({
        render: () => {
          const apply = (props: {
            items: EmojiEntry[];
            clientRect?: (() => DOMRect | null) | null;
            command: (e: EmojiEntry) => void;
          }) =>
            setSug({
              items: props.items.map((e) => ({
                id: e.emoji,
                label: `${e.emoji}  :${e.names[0]}:`,
              })),
              index: 0,
              rect: props.clientRect?.() ?? null,
              select: (item) => {
                const e = props.items.find((x) => x.emoji === item.id);
                if (e) props.command(e);
              },
              label: "Emoji suggestions",
            });
          return { onStart: apply, onUpdate: apply, onKeyDown: suggestKeyDown, onExit };
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
      handleDOMEvents: {
        // Links öffnen wie in Linear: Klick folgt dem Link (intern ohne Neuladen), Strg/Cmd- oder
        // Mittelklick in einem neuen Tab.
        click: (_view, event) => {
          if (event.button !== 0) return false;
          const href = linkTarget(event.target);
          if (!href) return false;
          if (!openLink(href, routerRef.current, { newTab: event.ctrlKey || event.metaKey }))
            return false;
          event.preventDefault();
          return true;
        },
        auxclick: (_view, event) => {
          if (event.button !== 1) return false;
          const href = linkTarget(event.target);
          if (!href || !openLink(href, undefined, { newTab: true })) return false;
          event.preventDefault();
          return true;
        },
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
    onSelectionUpdate: ({ editor: ed }) => {
      if (!inlineRef.current || ed.state.selection.empty) return setSelBox(null);
      const c = ed.view.coordsAtPos(ed.state.selection.to);
      setSelBox({ left: c.left, top: c.bottom + 4 });
    },
  });

  // Markierungen neu zeichnen, wenn sich die Kommentare oder die aktive Auswahl ändern.
  const marksKey = inline ? `${inline.activeId}|${JSON.stringify(inline.marks)}` : "";
  useEffect(() => {
    if (!editor || editor.isDestroyed || !inline) return;
    editor.view.dispatch(editor.state.tr.setMeta(inlineCommentsKey, true));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, marksKey]);

  // Nachgeladene Tickets/Teams: Chips neu zeichnen.
  useEffect(() => {
    if (!editor || !refStore) return;
    return refStore.subscribe(() => {
      if (!editor.isDestroyed) editor.view.dispatch(editor.state.tr.setMeta(issueRefsKey, true));
    });
  }, [editor, refStore]);

  return (
    <div className={cn("lineal-prose relative", className)}>
      <EditorContent editor={editor} />
      {inline && selBox && editor && (
        <button
          type="button"
          aria-label="Comment on selection"
          className="fixed z-[70] rounded-md border border-border-strong bg-popover px-2 py-1 text-xs shadow-popover hover:bg-surface-hover"
          style={{ left: selBox.left, top: selBox.top }}
          // Die Auswahl muss erhalten bleiben (kein Fokuswechsel).
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            const a = selectionAnchor(editor.state);
            setSelBox(null);
            if (a) inline.onComment(a);
          }}
        >
          Comment
        </button>
      )}
      {suggestion && suggestion.items.length > 0 && (
        <ul
          role="listbox"
          aria-label={suggestion.label}
          className="fixed z-[70] min-w-40 rounded-md border border-border-strong bg-popover p-1 shadow-popover"
          style={{
            left: suggestion.rect?.left ?? 0,
            top: (suggestion.rect?.bottom ?? 0) + 4,
          }}
        >
          {suggestion.items.map((item, i) => (
            <li
              key={item.id}
              role="option"
              aria-selected={i === suggestion.index}
              className={cn(
                "cursor-default rounded px-2 py-1 text-sm",
                i === suggestion.index && "bg-surface-hover",
              )}
              onMouseDown={(e) => {
                e.preventDefault();
                suggestion.select(item);
              }}
            >
              {item.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
