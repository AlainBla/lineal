/**
 * Bildansicht in groß (LIN-21): eine globale Lightbox für Bilder in Beschreibung, Kommentaren und
 * Anhängen; Zustand und `openLightbox` liegen in `image-lightbox-store`. Eingehängt einmal im
 * `IssuesHost`. Bei mehreren Bildern einer Liste lässt sich per Pfeil oder Pfeiltaste blättern
 * (LIN-56).
 */
import { ChevronLeft, ChevronRight, Download, X } from "lucide-react";
import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogTitle } from "@/components/ui/dialog";

import {
  closeLightbox,
  downloadUrl,
  nextLightbox,
  prevLightbox,
  useLightboxGallery,
} from "./image-lightbox-store";

export function ImageLightbox() {
  const gallery = useLightboxGallery();
  const index = gallery ? gallery.index : 0;
  const image = gallery ? gallery.images[index] : null;
  const count = gallery?.images.length ?? 0;
  const hasPrev = !!gallery && index > 0;
  const hasNext = !!gallery && index < count - 1;
  const contentRef = useRef<HTMLDivElement>(null);

  // Pfeiltasten blättern, solange die Lightbox offen ist (LIN-56). Am `window`, nicht am Dialog-
  // Inhalt: erreicht ein Pfeil-Knopf am Ende den `disabled`-Zustand, verliert der Inhalt den Fokus
  // (an `body`) und ein Content-Listener würde nicht mehr auslösen. Gehandhabt wird nur, wenn der
  // Fokus im Dialog liegt oder (nach so einem Disable) auf `body` – nie, wenn ein anderes Element
  // den Fokus hat.
  useEffect(() => {
    if (!image) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const active = document.activeElement;
      const content = contentRef.current;
      if (active && active !== document.body && !content?.contains(active)) return;
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        prevLightbox();
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        nextLightbox();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [image]);

  return (
    <Dialog open={image != null} onOpenChange={(open) => !open && closeLightbox()}>
      {image && (
        <DialogContent
          ref={contentRef}
          aria-describedby={undefined}
          className="w-auto max-w-[95vw] border-0 bg-transparent shadow-none"
          // Ein Klick neben das Bild (auf den transparenten Rand) schließt ebenfalls.
          onClick={(e) => {
            if (e.target === e.currentTarget) closeLightbox();
          }}
        >
          <DialogTitle className="sr-only">{image.alt || image.filename || "Image"}</DialogTitle>
          <img
            src={image.src}
            alt={image.alt}
            className="block max-h-[90vh] max-w-[95vw] rounded-md bg-background object-contain"
          />
          {count > 1 && (
            <>
              <Button
                variant="outline"
                size="icon"
                className="absolute top-1/2 left-2 size-8 -translate-y-1/2 bg-background/90 shadow-sm"
                aria-label="Previous file"
                title="Previous"
                disabled={!hasPrev}
                onClick={() => prevLightbox()}
              >
                <ChevronLeft className="size-4" />
              </Button>
              <Button
                variant="outline"
                size="icon"
                className="absolute top-1/2 right-2 size-8 -translate-y-1/2 bg-background/90 shadow-sm"
                aria-label="Next file"
                title="Next"
                disabled={!hasNext}
                onClick={() => nextLightbox()}
              >
                <ChevronRight className="size-4" />
              </Button>
              <span className="absolute bottom-2 left-1/2 -translate-x-1/2 rounded-full bg-background/90 px-2 py-0.5 text-xs text-muted-foreground shadow-sm">
                {index + 1} / {count}
              </span>
            </>
          )}
          <div className="absolute top-2 right-2 flex gap-1">
            <Button
              variant="outline"
              size="icon"
              className="size-8 bg-background/90 shadow-sm"
              aria-label="Download image"
              title="Download"
              onClick={() => void downloadUrl(image.src, image.filename ?? image.alt)}
            >
              <Download className="size-4" />
            </Button>
            <DialogClose asChild>
              <Button
                variant="outline"
                size="icon"
                className="size-8 bg-background/90 shadow-sm"
                aria-label="Close"
                title="Close"
              >
                <X className="size-4" />
              </Button>
            </DialogClose>
          </div>
        </DialogContent>
      )}
    </Dialog>
  );
}
