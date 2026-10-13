import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FileText, Paperclip, Trash2, Upload } from "lucide-react";
import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import type { Issue } from "@/gql/graphql";
import { cn } from "@/lib/cn";

import { sameOrigin, uploadFile } from "./assets";
import { useIssueExtras } from "./hooks";
import { createAttachment, deleteAttachment, keys } from "./queries";
import { errorMessage, showToast } from "./toast-store";

/** Dateigröße als „12 KB“ / „3.4 MB“. */
export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/**
 * Anhänge des Issues (docs/03 §7): Liste, Upload per Button und per Drag & Drop. Uploads laufen
 * über `fileUpload` (PUT) und werden mit `attachmentCreate` an das Issue gehängt.
 */
export function AttachmentsSection({
  issue,
  readOnly = false,
}: {
  issue: Issue;
  readOnly?: boolean;
}) {
  const qc = useQueryClient();
  const extras = useIssueExtras(issue.id);
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  const [busy, setBusy] = useState(0);
  const refresh = () => qc.invalidateQueries({ queryKey: keys.issueDetails });

  const remove = useMutation({
    mutationFn: deleteAttachment,
    onSuccess: refresh,
    onError: (e) =>
      showToast({ message: errorMessage(e, "Could not remove the attachment."), tone: "error" }),
  });

  const upload = async (files: File[]) => {
    for (const file of files) {
      setBusy((n) => n + 1);
      try {
        const { assetUrl, filename } = await uploadFile(file);
        await createAttachment({ issueId: issue.id, url: assetUrl, title: filename });
        await refresh();
      } catch (e) {
        showToast({
          message: errorMessage(e, `Could not upload ${file.name}.`),
          tone: "error",
        });
      } finally {
        setBusy((n) => n - 1);
      }
    }
  };

  const attachments = extras.data?.attachments ?? [];
  if (attachments.length === 0 && readOnly) return null;

  return (
    <section
      aria-label="Attachments"
      className={cn(
        "flex flex-col gap-1.5 rounded-md",
        over && "outline-2 -outline-offset-2 outline-primary/60 outline-dashed",
      )}
      onDragOver={(e) => {
        if (readOnly || !e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        if (readOnly || e.dataTransfer.files.length === 0) return;
        e.preventDefault();
        void upload([...e.dataTransfer.files]);
      }}
    >
      <header className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Attachments</h2>
        {attachments.length > 0 && (
          <span className="text-xs text-muted-foreground">{attachments.length}</span>
        )}
        {!readOnly && (
          <>
            <input
              ref={input}
              type="file"
              multiple
              hidden
              aria-label="Attach files"
              data-testid="attachment-input"
              onChange={(e) => {
                const files = [...(e.target.files ?? [])];
                e.target.value = "";
                if (files.length > 0) void upload(files);
              }}
            />
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              disabled={busy > 0}
              onClick={() => input.current?.click()}
            >
              <Upload /> {busy > 0 ? "Uploading…" : "Upload"}
            </Button>
          </>
        )}
      </header>
      {attachments.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          {extras.isError
            ? "Could not load attachments."
            : "Drop files here or use Upload. Paste an image into the description to embed it."}
        </p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {attachments.map((a) => (
            <li key={a.id} className="group flex items-center gap-2 text-sm">
              {a.sourceType === "upload" ? (
                <FileText className="size-3.5 text-muted-foreground" aria-hidden />
              ) : (
                <Paperclip className="size-3.5 text-muted-foreground" aria-hidden />
              )}
              <a
                href={a.sourceType === "upload" ? sameOrigin(a.url) : a.url}
                target="_blank"
                rel="noopener noreferrer"
                className="min-w-0 truncate hover:underline"
              >
                {a.title}
              </a>
              {a.size != null && (
                <span className="text-xs text-muted-foreground">{formatBytes(a.size)}</span>
              )}
              {!readOnly && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="ml-auto size-6 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                  aria-label={`Remove attachment ${a.title}`}
                  onClick={() => remove.mutate(a.id)}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
