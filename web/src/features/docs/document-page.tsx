import { History } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import { useKnownUsers } from "@/features/issues/users";
import type { Document } from "@/gql/graphql";

import { useDocument, useDocumentHistory, useRestoreDocument, useSaveDocument } from "./queries";

const AUTOSAVE_MS = 1000;
type Status = "saved" | "dirty" | "saving" | "conflict" | "error";

function Editor({
  doc,
  docRef,
  onRestored,
}: {
  doc: Document;
  docRef: string;
  onRestored: () => void;
}) {
  const save = useSaveDocument(docRef);
  const users = useKnownUsers();
  const [title, setTitle] = useState(doc.title);
  const [status, setStatus] = useState<Status>("saved");
  const [showHistory, setShowHistory] = useState(false);
  const version = useRef(doc.contentVersion);
  const pending = useRef<{ title?: string; content?: string }>({});
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    version.current = doc.contentVersion;
  }, [doc.contentVersion]);

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const patch = pending.current;
    if (!patch.title && patch.content === undefined) return;
    pending.current = {};
    setStatus("saving");
    save.mutate(
      { id: doc.id, expectedVersion: version.current, patch },
      {
        onSuccess: (d) => {
          version.current = d.contentVersion;
          setStatus(Object.keys(pending.current).length ? "dirty" : "saved");
        },
        onError: (e) => {
          const msg = e instanceof Error ? e.message.toLowerCase() : "";
          setStatus(msg.includes("conflict") || msg.includes("version") ? "conflict" : "error");
        },
      },
    );
  }, [doc.id, save]);

  const queue = (patch: { title?: string; content?: string }) => {
    pending.current = { ...pending.current, ...patch };
    setStatus("dirty");
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(flush, AUTOSAVE_MS);
  };

  // Beim Verlassen ungespeicherte Änderungen noch senden.
  const flushRef = useRef(flush);
  useEffect(() => {
    flushRef.current = flush;
  });
  useEffect(() => () => flushRef.current(), []);

  const statusText: Record<Status, string> = {
    saved: "Saved",
    dirty: "Unsaved changes",
    saving: "Saving…",
    conflict: "Changed elsewhere – reload to continue",
    error: "Could not save",
  };

  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col overflow-auto">
        <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
          <span className="text-xs text-muted-foreground" role="status">
            {statusText[status]}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto"
            aria-pressed={showHistory}
            onClick={() => setShowHistory((v) => !v)}
          >
            <History /> History
          </Button>
        </header>
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6">
          <Input
            aria-label="Document title"
            value={title}
            onChange={(e) => {
              setTitle(e.target.value);
              queue({ title: e.target.value });
            }}
            className="h-10 border-transparent text-xl font-semibold"
          />
          <RichEditor
            value={doc.content}
            ariaLabel="Document content"
            users={users}
            placeholder="Write something…"
            onChange={(md) => queue({ content: md })}
            onBlur={flush}
          />
          {doc.warnings.length > 0 && (
            <p className="text-xs text-amber-500">{doc.warnings.join(" ")}</p>
          )}
        </div>
      </div>
      {showHistory && <HistoryPanel doc={doc} docRef={docRef} onRestored={onRestored} />}
    </div>
  );
}

function HistoryPanel({
  doc,
  docRef,
  onRestored,
}: {
  doc: Document;
  docRef: string;
  onRestored: () => void;
}) {
  const q = useDocumentHistory(doc.id, true);
  const restore = useRestoreDocument(docRef);
  return (
    <aside
      aria-label="Document history"
      className="w-72 shrink-0 overflow-auto border-l border-border"
    >
      <h2 className="border-b border-border px-3 py-2 text-sm font-medium">History</h2>
      {q.isLoading && <ListSkeleton rows={4} />}
      {q.isError && <ErrorState error={q.error} title="Could not load history" />}
      <ul>
        {(q.data ?? []).map((h) => (
          <li
            key={h.id}
            className="flex items-center gap-2 border-b border-border px-3 py-2 text-sm"
          >
            <span className="min-w-0 flex-1">
              <span className="block truncate">{h.title ?? doc.title}</span>
              <span className="text-xs text-muted-foreground">
                {h.kind} · {relativeTime(h.contentSnapshotAt)}
              </span>
            </span>
            <Button
              variant="ghost"
              size="sm"
              disabled={restore.isPending}
              aria-label={`Restore version from ${h.contentSnapshotAt}`}
              onClick={() =>
                restore.mutate({ documentId: doc.id, historyId: h.id }, { onSuccess: onRestored })
              }
            >
              Restore
            </Button>
          </li>
        ))}
      </ul>
    </aside>
  );
}

/** Dokument-Editor (docs/06, F15): TipTap-Editor mit Autosave und Versionshistorie. */
export function DocumentPage({ docRef }: { docRef: string }) {
  const q = useDocument(docRef);
  const [nonce, setNonce] = useState(0);
  useEffect(() => {
    if (!q.data) return;
    const prev = document.title;
    document.title = `${q.data.title} · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [q.data]);
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError || !q.data)
    return (
      <ErrorState
        error={q.error}
        title="Could not load document"
        onRetry={() => void q.refetch()}
      />
    );
  // Nach „Restore“ lädt der Editor den wiederhergestellten Inhalt neu (Autosave würde sonst den Cursor verlieren).
  return (
    <Editor
      key={`${q.data.id}:${nonce}`}
      doc={q.data}
      docRef={docRef}
      onRestored={() => setNonce((n) => n + 1)}
    />
  );
}
