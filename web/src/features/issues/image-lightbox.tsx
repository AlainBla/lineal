/**
 * Bildansicht in groß (LIN-21): eine globale Lightbox für Bilder in Beschreibung, Kommentaren und
 * Anhängen; Zustand und `openLightbox` liegen in `image-lightbox-store`. Eingehängt einmal im
 * `IssuesHost`.
 */
import { Download, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogTitle } from "@/components/ui/dialog";

import { closeLightbox, downloadUrl, useLightboxImage } from "./image-lightbox-store";

export function ImageLightbox() {
  const image = useLightboxImage();
  return (
    <Dialog open={image != null} onOpenChange={(open) => !open && closeLightbox()}>
      {image && (
        <DialogContent
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
