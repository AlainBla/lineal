/**
 * Dokumentliste an einem Ort (Projekt „Resources“, Initiative, Team, Issue, Cycle; F.05): anlegen
 * (leer oder aus einem Template), öffnen, in den Papierkorb legen und aus „Recently deleted“
 * wiederherstellen. Die Rechte prüft der Server; Fehler erscheinen als Hinweis.
 */
import { Link, useNavigate } from "@tanstack/react-router";
import { FileText, Plus, Trash2, Undo2 } from "lucide-react";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import { relativeTime } from "@/features/issues/model";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";

import {
  type DocScope,
  useCreateDocument,
  useDocuments,
  useDocumentTemplates,
  useTrashDocument,
} from "./list-queries";

function NewDocumentForm({
  scope,
  teamId,
  onClose,
}: {
  scope: DocScope;
  /** Team für die Template-Auswahl (geerbte und Workspace-Templates). */
  teamId?: string;
  onClose: () => void;
}) {
  const session = useSession();
  const navigate = useNavigate();
  const templates = useDocumentTemplates(teamId);
  const create = useCreateDocument();
  const [title, setTitle] = useState("");
  const [templateId, setTemplateId] = useState("");
  const ws = session?.workspace.urlKey ?? "";
  const canSubmit = (title.trim() !== "" || templateId !== "") && !create.isPending;
  return (
    <form
      aria-label="New document"
      className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (!canSubmit) return;
        create.mutate(
          {
            scope,
            title: title.trim() || undefined,
            templateId: templateId || undefined,
          },
          {
            onSuccess: (doc) => {
              onClose();
              void navigate({ to: `/${ws}/document/${doc.slugId}` as "/" });
            },
          },
        );
      }}
    >
      <label className="flex items-center gap-1 text-sm">
        <span className="sr-only">Template</span>
        <select
          aria-label="Template"
          value={templateId}
          onChange={(e) => setTemplateId(e.target.value)}
          className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
        >
          <option value="">Blank document</option>
          {(templates.data ?? []).map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </label>
      <Input
        aria-label="Document name"
        placeholder={templateId ? "Title (optional, template title by default)" : "Document title"}
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        className="max-w-xs"
        autoFocus
      />
      <Button type="submit" size="sm" disabled={!canSubmit}>
        Create document
      </Button>
      <Button type="button" size="sm" variant="ghost" onClick={onClose}>
        Cancel
      </Button>
      {create.isError && (
        <p role="alert" className="w-full text-xs text-destructive">
          {errorText(create.error)}
        </p>
      )}
    </form>
  );
}

export function DocumentsSection({
  ws,
  scope,
  teamId,
}: {
  ws: string;
  scope: DocScope;
  teamId?: string;
}) {
  const [creating, setCreating] = useState(false);
  const [showTrash, setShowTrash] = useState(false);
  const live = useDocuments(scope, false);
  const trash = useDocuments(scope, true);
  const trashMut = useTrashDocument();
  const q = showTrash ? trash : live;
  return (
    <section aria-label="Documents">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1">
        <h2 className="text-sm font-medium">{showTrash ? "Recently deleted" : "Documents"}</h2>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          aria-pressed={showTrash}
          onClick={() => setShowTrash((v) => !v)}
        >
          <Trash2 /> Recently deleted
        </Button>
        {!showTrash && (
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus /> New document
          </Button>
        )}
      </div>
      {creating && !showTrash && (
        <NewDocumentForm scope={scope} teamId={teamId} onClose={() => setCreating(false)} />
      )}
      {q.isLoading && <ListSkeleton rows={3} />}
      {q.isError && (
        <ErrorState
          error={q.error}
          title="Could not load documents"
          onRetry={() => void q.refetch()}
        />
      )}
      {q.data && q.data.length === 0 && (
        <EmptyState
          title={showTrash ? "Nothing deleted" : "No documents"}
          description={showTrash ? undefined : "Create a document or start from a template."}
        />
      )}
      <ul className="divide-y divide-border">
        {(q.data ?? []).map((d) => (
          <li key={d.id} className="flex items-center gap-3 px-3 py-2 text-sm">
            <FileText className="size-4 text-muted-foreground" aria-hidden />
            {showTrash ? (
              <span className="min-w-0 flex-1 truncate">{d.title}</span>
            ) : (
              <Link
                to={`/${ws}/document/${d.slugId}` as "/"}
                className="min-w-0 flex-1 truncate hover:underline"
              >
                {d.icon ? `${d.icon} ` : ""}
                {d.title}
              </Link>
            )}
            <span className="text-xs text-muted-foreground">{relativeTime(d.updatedAt)}</span>
            {showTrash ? (
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Restore ${d.title}`}
                disabled={trashMut.isPending}
                onClick={() => trashMut.mutate({ id: d.id, restore: true })}
              >
                <Undo2 /> Restore
              </Button>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Delete ${d.title}`}
                disabled={trashMut.isPending}
                onClick={() => {
                  if (window.confirm(`Move "${d.title}" to Recently deleted?`))
                    trashMut.mutate({ id: d.id, restore: false });
                }}
              >
                <Trash2 /> Delete
              </Button>
            )}
          </li>
        ))}
      </ul>
      {trashMut.isError && (
        <p role="alert" className="px-3 py-2 text-xs text-destructive">
          {errorText(trashMut.error)}
        </p>
      )}
    </section>
  );
}
