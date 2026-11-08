/**
 * Markdown-Vorschau (LIN-43): holt den Text eines `.md`-Anhangs von der signierten same-origin-URL
 * und rendert ihn schreibgeschützt über den `RichEditor`. Einmal im `IssuesHost` eingehängt.
 */
import { Download, X } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogTitle } from "@/components/ui/dialog";

import { downloadUrl } from "./image-lightbox-store";
import { closeMarkdownPreview, useMarkdownPreview } from "./markdown-preview-store";
import { RichEditor } from "./rich-editor";

/** Größte Datei, die wir als Vorschau laden (reiner Text; alles darüber bleibt Download). */
const MAX_PREVIEW_BYTES = 2 * 1024 * 1024;

type Result = { status: "ready"; text: string } | { status: "error"; message: string };

export function MarkdownPreview() {
  const preview = useMarkdownPreview();
  // Das Ergebnis trägt die URL, für die es gilt; passt sie nicht zur aktuellen Vorschau, wird noch
  // geladen. So wird der Ladezustand abgeleitet statt synchron im Effekt gesetzt.
  const [loaded, setLoaded] = useState<{ src: string; result: Result } | null>(null);

  useEffect(() => {
    if (!preview) return;
    let cancelled = false;
    const src = preview.src;
    const done = (result: Result) => {
      if (!cancelled) setLoaded({ src, result });
    };
    (async () => {
      try {
        const res = await fetch(src, { credentials: "same-origin" });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const len = Number(res.headers.get("content-length") ?? "0");
        if (len > MAX_PREVIEW_BYTES) throw new Error("too-large");
        const text = await res.text();
        if (text.length > MAX_PREVIEW_BYTES) throw new Error("too-large");
        done({ status: "ready", text });
      } catch (e) {
        done({
          status: "error",
          message:
            e instanceof Error && e.message === "too-large"
              ? "This file is too large to preview. Download it instead."
              : "Could not load this file.",
        });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [preview]);

  const state: { status: "loading" } | Result =
    preview && loaded?.src === preview.src ? loaded.result : { status: "loading" };

  return (
    <Dialog open={preview != null} onOpenChange={(open) => !open && closeMarkdownPreview()}>
      {preview && (
        <DialogContent
          aria-describedby={undefined}
          className="flex max-h-[85vh] w-[min(92vw,56rem)] max-w-none flex-col gap-3 p-0"
        >
          <div className="flex items-center gap-2 border-b px-4 py-3">
            <DialogTitle className="min-w-0 truncate text-sm font-medium">
              {preview.title}
            </DialogTitle>
            <div className="ml-auto flex gap-1">
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                aria-label={`Download ${preview.title}`}
                title="Download"
                onClick={() => void downloadUrl(preview.src, preview.title)}
              >
                <Download className="size-4" />
              </Button>
              <DialogClose asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7"
                  aria-label="Close"
                  title="Close"
                >
                  <X className="size-4" />
                </Button>
              </DialogClose>
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-auto px-4 pb-4">
            {state.status === "loading" && (
              <p className="text-sm text-muted-foreground">Loading…</p>
            )}
            {state.status === "error" && (
              <p className="text-sm text-muted-foreground">{state.message}</p>
            )}
            {state.status === "ready" && (
              <RichEditor
                // Neuer Schlüssel je Datei: `value` wird nur beim Mounten übernommen.
                key={preview.src}
                value={state.text}
                ariaLabel={`Preview of ${preview.title}`}
                editable={false}
              />
            )}
          </div>
        </DialogContent>
      )}
    </Dialog>
  );
}
