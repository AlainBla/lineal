/**
 * Ressourcen eines Meilensteins oder Projekts (docs/04 §3.5): Dokumente, Dateien und Links in einem
 * Bereich. Die Rechte prüft der Server; Fehler erscheinen als Hinweis.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { FileText, Link2, Paperclip, Plus, X } from "lucide-react";
import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { useCreateDocument } from "@/features/docs/list-queries";
import { sameOrigin, uploadFile } from "@/features/issues/assets";
import { formatBytes } from "@/features/issues/attachments";
import { errorText } from "@/features/issues/error-text";
import type { Resources } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

const resourceKeys = {
  all: ["planning", "resources"] as const,
  scope: (projectId: string, milestoneId?: string) =>
    ["planning", "resources", projectId, milestoneId ?? null] as const,
};

/** Signierte Download-URLs laufen nach zehn Minuten ab; die Liste bleibt darunter. */
const REFRESH_MS = 5 * 60_000;

function useResources(projectId: string, milestoneId?: string) {
  return useQuery({
    queryKey: resourceKeys.scope(projectId, milestoneId),
    refetchInterval: REFRESH_MS,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ resources: Resources }, Record<string, unknown>>(
          `query PlanningResources($projectId: String!, $milestoneId: String) {
            resources(projectId: $projectId, milestoneId: $milestoneId) {
              documents { id title slugId }
              links { id url label }
              files { id title contentType size downloadUrl createdAt }
            }
          }`,
          { projectId, milestoneId },
          { signal },
        )
      ).resources,
  });
}

function useResourceMutations(projectId: string, milestoneId?: string) {
  const qc = useQueryClient();
  const scope = { projectId, milestoneId };
  const onSuccess = () => void qc.invalidateQueries({ queryKey: resourceKeys.all });
  return {
    addLink: useMutation({
      mutationFn: (v: { url: string; label: string }) =>
        gqlRequest(
          `mutation PlanningResourceLinkAdd($projectId: String!, $milestoneId: String, $url: String!, $label: String!) {
            resourceLinkAdd(projectId: $projectId, milestoneId: $milestoneId, url: $url, label: $label) { success }
          }`,
          { ...scope, ...v },
        ),
      onSuccess,
    }),
    removeLink: useMutation({
      mutationFn: (id: string) =>
        gqlRequest(
          `mutation PlanningResourceLinkRemove($projectId: String!, $id: String!) {
            resourceLinkRemove(projectId: $projectId, id: $id) { success }
          }`,
          { projectId, id },
        ),
      onSuccess,
    }),
    setDocument: useMutation({
      mutationFn: (v: { documentId: string; milestoneId?: string }) =>
        gqlRequest(
          `mutation PlanningResourceDocumentSet($projectId: String!, $documentId: String!, $milestoneId: String) {
            resourceDocumentSet(projectId: $projectId, documentId: $documentId, milestoneId: $milestoneId) { success }
          }`,
          { projectId, ...v },
        ),
      onSuccess,
    }),
    addFile: useMutation({
      mutationFn: async (file: File) => {
        const { assetUrl } = await uploadFile(file);
        return gqlRequest(
          `mutation PlanningResourceFileAdd($projectId: String!, $milestoneId: String, $assetUrl: String!, $title: String) {
            resourceFileAdd(projectId: $projectId, milestoneId: $milestoneId, assetUrl: $assetUrl, title: $title) { success }
          }`,
          { ...scope, assetUrl, title: file.name },
        );
      },
      onSuccess,
    }),
    removeFile: useMutation({
      mutationFn: (id: string) =>
        gqlRequest(
          `mutation PlanningResourceFileRemove($projectId: String!, $id: String!) {
            resourceFileRemove(projectId: $projectId, id: $id) { success }
          }`,
          { projectId, id },
        ),
      onSuccess,
    }),
  };
}

const fileMeta = (contentType?: string | null, size?: number | null) =>
  `${contentType?.startsWith("image/") ? "Image" : "File"}, ${formatBytes(size)}`;

function Row({
  icon,
  meta,
  children,
  removeLabel,
  onRemove,
}: {
  icon: React.ReactNode;
  meta: string;
  children: React.ReactNode;
  removeLabel: string;
  onRemove: () => void;
}) {
  return (
    <li className="group flex items-center gap-2 rounded-md px-1 py-1 hover:bg-surface-hover">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-surface-active text-muted-foreground [&_svg]:size-3.5">
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm">{children}</span>
        <span className="text-xs text-muted-foreground">{meta}</span>
      </span>
      <Button
        variant="ghost"
        size="icon"
        aria-label={removeLabel}
        className="opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"
        onClick={onRemove}
      >
        <X />
      </Button>
    </li>
  );
}

type Adding = "new-document" | "existing-document" | "link" | null;

export function ResourcesSection({
  ws,
  projectId,
  milestoneId,
}: {
  ws: string;
  projectId: string;
  /** Fehlt er, zeigt der Bereich die Ressourcen des Projekts selbst. */
  milestoneId?: string;
}) {
  const q = useResources(projectId, milestoneId);
  const projectDocs = useResources(projectId);
  const m = useResourceMutations(projectId, milestoneId);
  const createDocument = useCreateDocument();
  const navigate = useNavigate();
  const [adding, setAdding] = useState<Adding>(null);
  const [title, setTitle] = useState("");
  const [url, setUrl] = useState("");
  const fileInput = useRef<HTMLInputElement>(null);
  const failed =
    m.addLink.error ??
    m.removeLink.error ??
    m.setDocument.error ??
    m.addFile.error ??
    m.removeFile.error ??
    createDocument.error;
  const close = () => {
    setAdding(null);
    setTitle("");
    setUrl("");
  };
  // Am Projekt listet die Dokumentliste darüber die Dokumente.
  const documents = milestoneId ? (q.data?.documents ?? []) : [];
  const empty = q.data && !documents.length && !q.data.links.length && !q.data.files.length;
  return (
    <section aria-label="Resources" className="flex flex-col gap-1">
      <div className="flex items-center justify-between">
        <h2 className="text-xs font-medium text-muted-foreground">Resources</h2>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="Add resource">
              <Plus />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {milestoneId && (
              <>
                <DropdownMenuItem onSelect={() => setAdding("new-document")}>
                  <FileText /> New document
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => setAdding("existing-document")}>
                  <FileText /> Existing document…
                </DropdownMenuItem>
              </>
            )}
            <DropdownMenuItem onSelect={() => fileInput.current?.click()}>
              <Paperclip /> Upload file
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => setAdding("link")}>
              <Link2 /> Add link
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <input
        ref={fileInput}
        type="file"
        aria-label="Upload file"
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) m.addFile.mutate(file);
        }}
      />
      {adding === "link" && (
        <form
          aria-label="Add link"
          className="flex flex-col gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            if (!url.trim()) return;
            m.addLink.mutate(
              { url: url.trim(), label: title.trim() || url.trim() },
              { onSuccess: close },
            );
          }}
        >
          <Input
            aria-label="Link URL"
            placeholder="https://…"
            value={url}
            autoFocus
            onChange={(e) => setUrl(e.target.value)}
          />
          <Input
            aria-label="Link title"
            placeholder="Title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
          <div className="flex justify-end gap-1">
            <Button type="button" variant="ghost" size="sm" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={!url.trim() || m.addLink.isPending}>
              Add
            </Button>
          </div>
        </form>
      )}
      {adding === "new-document" && (
        <form
          aria-label="New document"
          className="flex items-center gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            if (!title.trim()) return;
            createDocument.mutate(
              { scope: { projectId }, title: title.trim() },
              {
                onSuccess: async (doc) => {
                  if (milestoneId) {
                    await m.setDocument.mutateAsync({ documentId: doc.id, milestoneId });
                  }
                  close();
                  void navigate({ to: `/${ws}/document/${doc.slugId}` as "/" });
                },
              },
            );
          }}
        >
          <Input
            aria-label="Document title"
            placeholder="Document title"
            value={title}
            autoFocus
            onChange={(e) => setTitle(e.target.value)}
          />
          <Button type="submit" size="sm" disabled={!title.trim() || createDocument.isPending}>
            Create
          </Button>
        </form>
      )}
      {adding === "existing-document" && (
        <select
          aria-label="Existing document"
          className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
          value=""
          onChange={(e) => {
            if (e.target.value) {
              m.setDocument.mutate(
                { documentId: e.target.value, milestoneId },
                { onSuccess: close },
              );
            }
          }}
        >
          <option value="">Choose a project document…</option>
          {(projectDocs.data?.documents ?? []).map((d) => (
            <option key={d.id} value={d.id}>
              {d.title || "Untitled"}
            </option>
          ))}
        </select>
      )}
      {q.isError && <p className="text-xs text-destructive">{errorText(q.error)}</p>}
      {empty && !adding && (
        <p className="text-sm text-muted-foreground">No documents, files or links yet.</p>
      )}
      <ul className="flex flex-col">
        {documents.map((d) => (
          <Row
            key={d.id}
            icon={<FileText />}
            meta="Document"
            removeLabel={`Remove ${d.title || "document"}`}
            onRemove={() => m.setDocument.mutate({ documentId: d.id })}
          >
            <Link to={`/${ws}/document/${d.slugId}` as "/"} className="hover:underline">
              {d.title || "Untitled"}
            </Link>
          </Row>
        ))}
        {q.data?.files.map((f) => (
          <Row
            key={f.id}
            icon={<Paperclip />}
            meta={fileMeta(f.contentType, f.size)}
            removeLabel={`Delete ${f.title}`}
            onRemove={() => {
              if (window.confirm(`Delete file "${f.title}"?`)) m.removeFile.mutate(f.id);
            }}
          >
            <a href={sameOrigin(f.downloadUrl)} download={f.title} className="hover:underline">
              {f.title}
            </a>
          </Row>
        ))}
        {q.data?.links.map((l) => (
          <Row
            key={l.id}
            icon={<Link2 />}
            meta="Link"
            removeLabel={`Remove ${l.label}`}
            onRemove={() => m.removeLink.mutate(l.id)}
          >
            <a href={l.url} target="_blank" rel="noreferrer" className="hover:underline">
              {l.label}
            </a>
          </Row>
        ))}
      </ul>
      {m.addFile.isPending && <p className="text-xs text-muted-foreground">Uploading…</p>}
      {failed && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(failed)}
        </p>
      )}
    </section>
  );
}
