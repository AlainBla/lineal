/** Seitenleiste der Inline-Kommentar-Threads eines Dokuments (docs/06 §1.8, F.06). */
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { CommentComposer, CommentThread, groupThreads } from "@/features/comments/comments-section";
import type { InlineAnchor, useCommentMutations } from "@/features/comments/queries";
import type { Comment } from "@/gql/graphql";

export function InlineThreadsPanel({
  comments,
  mutations,
  activeId,
  onSelect,
  pending,
  onCancelPending,
}: {
  comments: Comment[];
  mutations: ReturnType<typeof useCommentMutations>;
  activeId: string | null;
  onSelect: (id: string) => void;
  /** Auswahl, zu der gerade ein neuer Thread verfasst wird. */
  pending: InlineAnchor | null;
  onCancelPending: () => void;
}) {
  const [showResolved, setShowResolved] = useState(false);
  const threads = groupThreads(comments).filter(({ root }) => root.quotedText != null);
  const open = threads.filter((t) => !t.root.resolvedAt);
  const resolved = threads.filter((t) => t.root.resolvedAt);
  return (
    <aside
      aria-label="Comment threads"
      className="flex w-80 shrink-0 flex-col gap-3 overflow-auto border-l border-border p-3"
    >
      <h2 className="text-sm font-medium">Comments</h2>
      {pending && (
        <div className="flex flex-col gap-2">
          <blockquote className="border-l-2 pl-2 text-xs text-muted-foreground">
            {pending.quotedText}
          </blockquote>
          <CommentComposer
            mutations={mutations}
            placeholder="Comment on the selected text…"
            ariaLabel="Inline comment"
            extra={pending}
            onDone={onCancelPending}
          />
        </div>
      )}
      {open.length === 0 && !pending && (
        <p className="text-xs text-muted-foreground">
          Select text in the document and choose “Comment” to start a thread.
        </p>
      )}
      <ol className="flex flex-col gap-2" aria-label="Open threads">
        {open.map(({ root, replies }) => (
          <li key={root.id}>
            <CommentThread
              root={root}
              replies={replies}
              mutations={mutations}
              active={root.id === activeId}
              onSelect={() => onSelect(root.id)}
            />
          </li>
        ))}
      </ol>
      {resolved.length > 0 && (
        <>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={showResolved}
            onClick={() => setShowResolved((v) => !v)}
          >
            {showResolved ? "Hide" : "Show"} resolved ({resolved.length})
          </Button>
          {showResolved && (
            <ol className="flex flex-col gap-2" aria-label="Resolved threads">
              {resolved.map(({ root, replies }) => (
                <li key={root.id}>
                  <CommentThread root={root} replies={replies} mutations={mutations} />
                </li>
              ))}
            </ol>
          )}
        </>
      )}
    </aside>
  );
}
