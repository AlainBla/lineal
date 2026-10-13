import { Bell, BellOff, SmilePlus } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useMemo, useRef, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { Comment, Issue, IssueHistoryEntry } from "@/gql/graphql";
import { cn } from "@/lib/cn";

import { useComments, useCreateComment, useIssueExtras, useReactions } from "./hooks";
import { Avatar } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { describeHistory, type HistoryResolver } from "./history";
import { relativeTime } from "./model";
import { keys, setSubscription } from "./queries";
import { useSingleFlight } from "@/lib/use-single-flight";

import { RichEditor } from "./rich-editor";
import { ErrorState } from "./states";
import { errorMessage, showToast } from "./toast-store";
import { useKnownUsers } from "./users";

const QUICK_EMOJI = ["👍", "❤️", "🎉", "👀", "🚀"];

function Reactions({
  comment,
  issueId,
  myReactions,
}: {
  comment: Comment;
  issueId: string;
  myReactions: React.MutableRefObject<Map<string, string>>;
}) {
  const session = useSession();
  const { add, remove } = useReactions(issueId);
  const [picking, setPicking] = useState(false);
  const toggle = (emoji: string, mine: boolean) => {
    const key = `${comment.id}:${emoji}`;
    const known = myReactions.current.get(key);
    const fail = (e: unknown) => showToast({ message: errorMessage(e), tone: "error" });
    if (mine && known) {
      remove.mutate(known, {
        onSuccess: () => myReactions.current.delete(key),
        onError: fail,
      });
    } else if (!mine) {
      add.mutate(
        { commentId: comment.id, emoji },
        { onSuccess: (r) => myReactions.current.set(key, r.id), onError: fail },
      );
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-1">
      {comment.reactionData.map((r) => {
        const mine = !!session && r.userIds.includes(session.user.id);
        return (
          <button
            key={r.emoji}
            type="button"
            aria-pressed={mine}
            aria-label={`${r.emoji} ${r.count}`}
            onClick={() => toggle(r.emoji, mine)}
            className={cn(
              "rounded-full border px-2 text-xs",
              mine ? "border-primary/50 bg-primary/10" : "border-border hover:bg-surface-hover",
            )}
          >
            {r.emoji} {r.count}
          </button>
        );
      })}
      <span className="relative">
        <Button
          variant="ghost"
          size="icon"
          className="size-6"
          aria-label="Add reaction"
          aria-expanded={picking}
          onClick={() => setPicking((p) => !p)}
        >
          <SmilePlus className="size-3.5" />
        </Button>
        {picking && (
          <span className="absolute top-7 left-0 z-10 flex gap-1 rounded-md border border-border-strong bg-popover p-1 shadow-popover">
            {QUICK_EMOJI.map((e) => (
              <button
                key={e}
                type="button"
                className="rounded px-1 hover:bg-surface-hover"
                aria-label={`React with ${e}`}
                onClick={() => {
                  setPicking(false);
                  const mine = comment.reactionData.some(
                    (r) => r.emoji === e && !!session && r.userIds.includes(session.user.id),
                  );
                  if (!mine) toggle(e, false);
                }}
              >
                {e}
              </button>
            ))}
          </span>
        )}
      </span>
    </div>
  );
}

function CommentComposer({
  issueId,
  parentId,
  placeholder,
  ariaLabel,
  onDone,
}: {
  issueId: string;
  parentId?: string;
  placeholder: string;
  ariaLabel: string;
  onDone?: () => void;
}) {
  const create = useCreateComment(issueId);
  const users = useKnownUsers();
  const session = useSession();
  const [body, setBody] = useState("");
  const [nonce, setNonce] = useState(0);
  const mentionUsers = useMemo(
    () =>
      session
        ? [
            { id: session.user.id, name: session.user.name },
            ...users.filter((u) => u.id !== session.user.id),
          ]
        : users,
    [users, session],
  );
  const empty = body.trim() === "";
  const singleFlight = useSingleFlight();
  const submit = () => {
    if (empty) return;
    // Ref-Sperre: Doppelklick/mehrfaches Strg+Enter legt nur einen Kommentar an (B4.13).
    void singleFlight(async () => {
      try {
        await create.mutateAsync({ body, parentId });
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
        users={mentionUsers}
        // Antwortfeld sofort fokussieren, sonst lösen getippte Zeichen globale Shortcuts aus (B4.12).
        autoFocus={!!onDone}
        onChange={setBody}
        onSubmit={submit}
      />
      <div className="mt-2 flex justify-end">
        <Button size="sm" onClick={submit} disabled={empty || create.isPending}>
          {parentId ? "Reply" : "Comment"}
        </Button>
      </div>
    </div>
  );
}

function CommentCard({
  comment,
  replies,
  issueId,
  users,
  myReactions,
}: {
  comment: Comment;
  replies: Comment[];
  issueId: string;
  users: ReturnType<typeof useKnownUsers>;
  myReactions: React.MutableRefObject<Map<string, string>>;
}) {
  const [replying, setReplying] = useState(false);
  const author = comment.user?.displayName || comment.user?.name || "Unknown user";
  return (
    <article
      id={`comment-${comment.id}`}
      className="rounded-lg border border-border p-3"
      aria-label={`Comment by ${author}`}
    >
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
      </header>
      <RichEditor value={comment.body} editable={false} ariaLabel="Comment text" users={users} />
      <div className="mt-1">
        <Reactions comment={comment} issueId={issueId} myReactions={myReactions} />
      </div>
      {replies.length > 0 && (
        <div className="mt-2 flex flex-col gap-2 border-l border-border pl-3">
          {replies.map((r) => (
            <CommentCard
              key={r.id}
              comment={r}
              replies={[]}
              issueId={issueId}
              users={users}
              myReactions={myReactions}
            />
          ))}
        </div>
      )}
      <div className="mt-2">
        {replying ? (
          <CommentComposer
            issueId={issueId}
            parentId={comment.parentId ?? comment.id}
            placeholder="Leave a reply…"
            ariaLabel="Leave a reply"
            onDone={() => setReplying(false)}
          />
        ) : (
          !comment.parentId && (
            <Button variant="ghost" size="sm" onClick={() => setReplying(true)}>
              Leave a reply…
            </Button>
          )
        )}
      </div>
    </article>
  );
}

type TimelineItem =
  | { kind: "comment"; at: string; comment: Comment }
  | { kind: "history"; at: string; entry: IssueHistoryEntry };

function HistoryLine({ entry, resolver }: { entry: IssueHistoryEntry; resolver: HistoryResolver }) {
  const d = describeHistory(entry, resolver);
  if (d.phrases.length === 0) return null;
  return (
    <li
      className="flex items-baseline gap-2 text-xs text-muted-foreground"
      aria-label={`${d.actor} ${d.phrases.join(", ")}`}
    >
      <span
        aria-hidden
        className="size-1.5 shrink-0 translate-y-[-1px] rounded-full bg-border-strong"
      />
      <span className="min-w-0">
        <span className="font-medium text-foreground">{d.actor}</span> {d.phrases.join(", ")}
        {d.via && <span className="text-subtle-foreground"> (via {d.via})</span>}
        <time
          dateTime={d.createdAt}
          className="text-subtle-foreground"
          title={new Date(d.createdAt).toLocaleString()}
        >
          {" "}
          · {relativeTime(d.createdAt)}
        </time>
      </span>
    </li>
  );
}

/** Activity-Abschnitt: Abo-Schalter, Verlauf und Kommentar-Threads chronologisch gemischt, Kommentar-Editor. */
export function ActivitySection({ issue, lookup }: { issue: Issue; lookup: IssueLookup }) {
  const comments = useComments(issue.id);
  const users = useKnownUsers();
  const myReactions = useRef(new Map<string, string>());
  const extras = useIssueExtras(issue.id);
  const qc = useQueryClient();
  // Optimistischer Zwischenstand; sonst gilt der Abo-Zustand des Servers (`Issue.subscribed`).
  const [override, setOverride] = useState<boolean | null>(null);
  const subscribed = override ?? extras.data?.subscribed ?? false;
  const session = useSession();

  const resolver = useMemo<HistoryResolver>(
    () => ({
      user: (id) => lookup.assigneeName(id),
      state: (id) => lookup.state(id)?.name ?? null,
      label: (id) => lookup.label(id)?.name ?? null,
      project: (id) => lookup.projectName(id),
      cycle: (id) => lookup.cycleName(id),
      team: (id) => session?.teams.find((t) => t.id === id)?.name ?? null,
    }),
    [lookup, session],
  );

  const { timeline, repliesByParent } = useMemo(() => {
    const all = [...(comments.data ?? [])].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    const by = new Map<string, Comment[]>();
    for (const c of all) {
      if (c.parentId) by.set(c.parentId, [...(by.get(c.parentId) ?? []), c]);
    }
    const items: TimelineItem[] = [
      ...all
        .filter((c) => !c.parentId)
        .map((comment): TimelineItem => ({ kind: "comment", at: comment.createdAt, comment })),
      ...(extras.data?.history ?? []).map((entry): TimelineItem => ({
        kind: "history",
        at: entry.createdAt,
        entry,
      })),
    ];
    // `sort` ist stabil: bei gleichem Zeitstempel bleibt der Verlauf vor dem Kommentar.
    items.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
    return { timeline: items, repliesByParent: by };
  }, [comments.data, extras.data]);

  const toggleSub = () => {
    const next = !subscribed;
    setOverride(next);
    setSubscription(issue.id, next)
      .then(() => qc.invalidateQueries({ queryKey: keys.issueDetails }))
      .then(() => setOverride(null))
      .catch((e: unknown) => {
        setOverride(null);
        showToast({ message: errorMessage(e, "Could not change subscription."), tone: "error" });
      });
  };

  return (
    <section aria-label="Activity" className="flex flex-col gap-3">
      <header className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Activity</h2>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto"
          onClick={toggleSub}
          aria-pressed={subscribed}
          aria-label={subscribed ? "Unsubscribe from this issue" : "Subscribe to this issue"}
        >
          {subscribed ? <BellOff /> : <Bell />} {subscribed ? "Unsubscribe" : "Subscribe"}
        </Button>
      </header>
      {extras.isError && (
        <p role="alert" className="text-xs text-destructive">
          Could not load the activity history.
        </p>
      )}
      {comments.isError ? (
        <ErrorState
          title="Could not load comments"
          error={comments.error}
          onRetry={() => void comments.refetch()}
        />
      ) : (
        <ol className="flex flex-col gap-2" aria-label="Activity timeline">
          {timeline.map((item) =>
            item.kind === "history" ? (
              <HistoryLine key={`h-${item.entry.id}`} entry={item.entry} resolver={resolver} />
            ) : (
              <li key={`c-${item.comment.id}`}>
                <CommentCard
                  comment={item.comment}
                  replies={repliesByParent.get(item.comment.id) ?? []}
                  issueId={issue.id}
                  users={users}
                  myReactions={myReactions}
                />
              </li>
            ),
          )}
        </ol>
      )}
      {comments.isLoading && <Skeleton className="h-16 w-full" />}
      <CommentComposer
        issueId={issue.id}
        placeholder="Leave a comment…"
        ariaLabel="Leave a comment"
      />
    </section>
  );
}
