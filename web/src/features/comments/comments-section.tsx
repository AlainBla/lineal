/**
 * Kommentar-Bereich für Dokumente, Projekte, Initiativen und Status-Updates (F.06): Liste mit
 * Antworten, Anlegen, Bearbeiten/Löschen (eigene Kommentare; der Server prüft die Rechte erneut),
 * Auflösen und Scrollen zu `#comment-<id>` (Benachrichtigungslinks). Dieselbe Thread-Darstellung
 * dient der Seitenleiste der Inline-Kommentare im Dokument-Editor.
 */
import { useEffect, useMemo, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Avatar } from "@/features/issues/icons";
import { relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { ErrorState } from "@/features/issues/states";
import { errorMessage, showToast } from "@/features/issues/toast-store";
import { useKnownUsers } from "@/features/issues/users";
import type { Comment } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useSingleFlight } from "@/lib/use-single-flight";

import { type CommentTarget, useCommentMutations, useTargetComments } from "./queries";

type Mutations = ReturnType<typeof useCommentMutations>;

/** Gruppiert Antworten unter ihrer Wurzel (Server normalisiert die Thread-Wurzel). */
export function groupThreads(comments: Comment[]): { root: Comment; replies: Comment[] }[] {
  const sorted = [...comments].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  const replies = new Map<string, Comment[]>();
  for (const c of sorted) {
    if (c.parentId) replies.set(c.parentId, [...(replies.get(c.parentId) ?? []), c]);
  }
  return sorted
    .filter((c) => !c.parentId)
    .map((root) => ({ root, replies: replies.get(root.id) ?? [] }));
}

/** Scrollt zum Kommentar aus `#comment-<id>` und hebt ihn kurz hervor, sobald er gerendert ist. */
export function useScrollToCommentHash(ready: boolean): void {
  useEffect(() => {
    if (!ready) return;
    const m = /^#comment-(.+)$/.exec(window.location.hash);
    if (!m) return;
    const el = document.getElementById(`comment-${m[1]}`);
    if (!el) return;
    el.scrollIntoView({ block: "center" });
    el.classList.add("ring-2", "ring-primary");
    const t = setTimeout(() => el.classList.remove("ring-2", "ring-primary"), 3000);
    return () => clearTimeout(t);
  }, [ready]);
}

function Composer({
  mutations,
  parentId,
  placeholder,
  ariaLabel,
  onDone,
  extra,
}: {
  mutations: Mutations;
  parentId?: string;
  placeholder: string;
  ariaLabel: string;
  onDone?: () => void;
  /** Zusatzfelder (Inline-Anker) für das neue Wurzel-Kommentar. */
  extra?: { quotedText: string; anchorStart: string; anchorEnd: string };
}) {
  const users = useKnownUsers();
  const [body, setBody] = useState("");
  const [nonce, setNonce] = useState(0);
  const singleFlight = useSingleFlight();
  const empty = body.trim() === "";
  const submit = () => {
    if (empty) return;
    void singleFlight(async () => {
      try {
        await mutations.create.mutateAsync({ body, parentId, ...extra });
        setBody("");
        setNonce((n) => n + 1);
        onDone?.();
      } catch (e) {
        showToast({ message: errorMessage(e, "Could not post comment."), tone: "error" });
      }
    });
  };
  return (
    <div className="rounded-lg border border-border-strong p-2.5">
      <RichEditor
        key={nonce}
        value=""
        ariaLabel={ariaLabel}
        placeholder={placeholder}
        users={users}
        autoFocus={!!onDone || !!extra}
        onChange={setBody}
        onSubmit={submit}
      />
      <div className="mt-2 flex justify-end gap-2">
        {onDone && (
          <Button size="sm" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
        )}
        <Button size="sm" onClick={submit} disabled={empty || mutations.create.isPending}>
          {parentId ? "Reply" : "Comment"}
        </Button>
      </div>
    </div>
  );
}

function CommentBody({
  comment,
  mutations,
  isRoot,
  inlineRoot,
}: {
  comment: Comment;
  mutations: Mutations;
  isRoot: boolean;
  inlineRoot: boolean;
}) {
  const session = useSession();
  const users = useKnownUsers();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(comment.body);
  const author = comment.user?.displayName || comment.user?.name || "Unknown user";
  const mine = !!session && comment.userId === session.user.id;
  const fail = (e: unknown, fallback: string) =>
    showToast({ message: errorMessage(e, fallback), tone: "error" });
  return (
    <>
      <header className="mb-1 flex items-center gap-2 text-sm">
        <Avatar name={author} />
        <span className="font-medium">{author}</span>
        <a
          href={`#comment-${comment.id}`}
          className="text-xs text-subtle-foreground hover:underline"
          title={new Date(comment.createdAt).toLocaleString()}
        >
          {relativeTime(comment.createdAt)}
        </a>
        {comment.editedAt && <span className="text-xs text-subtle-foreground">(edited)</span>}
        {comment.resolvedAt && isRoot && (
          <span className="rounded bg-surface-active px-1.5 text-xs">Resolved</span>
        )}
      </header>
      {editing ? (
        <div className="rounded-lg border border-border-strong p-2.5">
          <RichEditor
            value={comment.body}
            ariaLabel="Edit comment"
            users={users}
            autoFocus
            onChange={setDraft}
          />
          <div className="mt-2 flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={draft.trim() === "" || mutations.update.isPending}
              onClick={() =>
                mutations.update.mutate(
                  { id: comment.id, body: draft },
                  {
                    onSuccess: () => setEditing(false),
                    onError: (e) => fail(e, "Could not save the comment."),
                  },
                )
              }
            >
              Save
            </Button>
          </div>
        </div>
      ) : (
        <RichEditor value={comment.body} editable={false} ariaLabel="Comment text" users={users} />
      )}
      {!editing && (
        <div className="mt-1 flex gap-1">
          {mine && (
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
              Edit
            </Button>
          )}
          {/* Der Wurzelkommentar eines Inline-Threads ist nicht löschbar (docs/06 §1.8). */}
          {mine && !inlineRoot && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                if (window.confirm("Delete this comment?"))
                  mutations.remove.mutate(comment.id, {
                    onError: (e) => fail(e, "Could not delete the comment."),
                  });
              }}
            >
              Delete
            </Button>
          )}
          {isRoot && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                mutations.resolve.mutate(
                  { id: comment.id, resolved: !comment.resolvedAt },
                  { onError: (e) => fail(e, "Could not change the thread.") },
                )
              }
            >
              {comment.resolvedAt ? "Reopen" : "Resolve"}
            </Button>
          )}
        </div>
      )}
    </>
  );
}

/** Ein Thread: Wurzel, Antworten und Antwortfeld. */
export function CommentThread({
  root,
  replies,
  mutations,
  active,
  onSelect,
}: {
  root: Comment;
  replies: Comment[];
  mutations: Mutations;
  active?: boolean;
  onSelect?: () => void;
}) {
  const [replying, setReplying] = useState(false);
  const inline = root.quotedText != null;
  const author = root.user?.displayName || root.user?.name || "Unknown user";
  return (
    <article
      id={`comment-${root.id}`}
      aria-label={`Comment by ${author}`}
      className={cn(
        "rounded-lg border p-3",
        active ? "border-primary" : "border-border",
        root.resolvedAt && "opacity-70",
      )}
      onClick={onSelect}
    >
      {inline && (
        <blockquote
          className={cn(
            "mb-2 border-l-2 pl-2 text-xs text-muted-foreground",
            root.anchorState === "detached" && "line-through",
          )}
        >
          {root.quotedText}
        </blockquote>
      )}
      <CommentBody comment={root} mutations={mutations} isRoot inlineRoot={inline} />
      {replies.length > 0 && (
        <div className="mt-2 flex flex-col gap-2 border-l border-border pl-3">
          {replies.map((r) => (
            <div key={r.id} id={`comment-${r.id}`} className="rounded">
              <CommentBody comment={r} mutations={mutations} isRoot={false} inlineRoot={false} />
            </div>
          ))}
        </div>
      )}
      <div className="mt-2">
        {replying ? (
          <Composer
            mutations={mutations}
            parentId={root.id}
            placeholder="Leave a reply…"
            ariaLabel="Leave a reply"
            onDone={() => setReplying(false)}
          />
        ) : (
          <Button variant="ghost" size="sm" onClick={() => setReplying(true)}>
            Leave a reply…
          </Button>
        )}
      </div>
    </article>
  );
}

export { Composer as CommentComposer };

/**
 * Kommentare eines Objekts. Bei Dokumenten zeigt `scope="regular"` nur normale Kommentare; die
 * Inline-Threads erscheinen in der Seitenleiste des Editors.
 */
export function CommentsSection({
  target,
  title = "Comments",
  scope = "all",
  refetchMs,
}: {
  target: CommentTarget;
  title?: string;
  scope?: "all" | "regular";
  refetchMs?: number;
}) {
  const q = useTargetComments(target, refetchMs);
  const mutations = useCommentMutations(target);
  const threads = useMemo(
    () =>
      groupThreads(q.data ?? []).filter(({ root }) => scope === "all" || root.quotedText == null),
    [q.data, scope],
  );
  useScrollToCommentHash(q.isSuccess);
  return (
    <section aria-label={title} className="flex flex-col gap-3">
      <h2 className="text-sm font-medium">{title}</h2>
      {q.isError ? (
        <ErrorState
          title="Could not load comments"
          error={q.error}
          onRetry={() => void q.refetch()}
        />
      ) : (
        <ol className="flex flex-col gap-2" aria-label={`${title} list`}>
          {threads.map(({ root, replies }) => (
            <li key={root.id}>
              <CommentThread root={root} replies={replies} mutations={mutations} />
            </li>
          ))}
        </ol>
      )}
      {q.isLoading && <Skeleton className="h-16 w-full" />}
      <Composer mutations={mutations} placeholder="Leave a comment…" ariaLabel="Leave a comment" />
    </section>
  );
}
