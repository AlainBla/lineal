import { Link } from "@tanstack/react-router";
import { BellOff, Check, Clock, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Notification } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useShortcutContext, useShortcuts } from "@/lib/shortcuts";
import { IssueDetailPage } from "@/features/issues/issue-detail";
import { relativeTime } from "@/features/issues/model";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";

import { bundleSummary, targetKind, typeLabel } from "./notification-types";
import { snoozePresets, useInboxAction, useNotifications } from "./queries";

/** Snooze-Auswahl: Vorgaben plus „Custom…“ (Datum/Uhrzeit); `open` steuert die Taste H. */
function SnoozeMenu({
  onPick,
  open,
  onOpenChange,
  children,
}: {
  onPick: (until: Date) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const [customOpen, setCustomOpen] = useState(false);
  const [custom, setCustom] = useState("");
  const customDate = custom ? new Date(custom) : null;
  const customValid = !!customDate && !Number.isNaN(customDate.getTime());
  return (
    <>
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {snoozePresets(new Date()).map((p) => (
            <DropdownMenuItem key={p.id} onSelect={() => onPick(p.until)}>
              {p.label}
            </DropdownMenuItem>
          ))}
          <DropdownMenuItem onSelect={() => setCustomOpen(true)}>Custom…</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={customOpen} onOpenChange={setCustomOpen}>
        <DialogContent className="max-w-sm">
          <form
            className="flex flex-col gap-3 p-4"
            onSubmit={(e) => {
              e.preventDefault();
              if (!customValid || !customDate || customDate.getTime() <= Date.now()) return;
              onPick(customDate);
              setCustomOpen(false);
            }}
          >
            <DialogTitle className="text-sm font-medium">Snooze until…</DialogTitle>
            <DialogDescription className="sr-only">Pick a date and time.</DialogDescription>
            <input
              type="datetime-local"
              aria-label="Snooze until"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
            />
            <Button type="submit" size="sm" disabled={!customValid}>
              Snooze
            </Button>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}

function Row({
  n,
  selected,
  onSelect,
}: {
  n: Notification;
  selected: boolean;
  onSelect: () => void;
}) {
  const unread = !n.readAt;
  const bundle = bundleSummary(n);
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      data-notification-id={n.id}
      onClick={onSelect}
      className={cn(
        "flex w-full flex-col gap-0.5 border-b border-border px-3 py-2 text-left text-sm outline-none hover:bg-surface-hover focus-visible:bg-surface-hover",
        selected && "bg-surface-active",
      )}
    >
      <span className="flex items-center gap-2">
        <span
          aria-label={unread ? "Unread" : "Read"}
          className={cn("size-1.5 shrink-0 rounded-full", unread ? "bg-primary" : "bg-transparent")}
        />
        {n.issueIdentifier && (
          <span className="text-xs text-muted-foreground">{n.issueIdentifier}</span>
        )}
        <span className={cn("truncate", unread && "font-semibold")}>{n.title}</span>
        <span
          data-testid="type-badge"
          className="shrink-0 rounded bg-surface-hover px-1 text-[10px] tracking-wide text-muted-foreground uppercase"
        >
          {typeLabel(n.type)}
        </span>
        {bundle.count > 1 && (
          <span
            data-testid="bundle-badge"
            title={bundle.labels.join(", ")}
            className="shrink-0 rounded-full bg-primary/15 px-1.5 text-[10px] font-medium text-primary"
          >
            {bundle.count}
          </span>
        )}
        <span className="ml-auto shrink-0 text-xs text-subtle-foreground">
          {relativeTime(n.createdAt)}
        </span>
      </span>
      <span className="truncate pl-3.5 text-xs text-muted-foreground">
        {n.actorName ? `${n.actorName} · ` : ""}
        {n.subtitle}
      </span>
    </button>
  );
}

/** Inbox (docs/08 §2.9, F13): Liste links, Bezugsobjekt rechts; Tastatur J/K, U, Entf, Alt+U. */
export function InboxPage({ ws }: { ws: string }) {
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [showSnoozed, setShowSnoozed] = useState(false);
  const q = useNotifications({ unreadOnly, showSnoozed });
  const act = useInboxAction();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snoozeOpen, setSnoozeOpen] = useState(false);
  const list = useMemo(() => q.data ?? [], [q.data]);
  const idx = list.findIndex((n) => n.id === selectedId);
  const selected = idx >= 0 ? list[idx]! : null;
  const unreadCount = list.filter((n) => !n.readAt).length;
  useShortcutContext("list");

  useEffect(() => {
    document.title = "Inbox · Lineal";
  }, []);

  // Öffnen einer Notification markiert sie als gelesen.
  const open = (n: Notification) => {
    setSelectedId(n.id);
    if (!n.readAt) act.mutate({ kind: "read", id: n.id, read: true });
  };
  const move = (d: 1 | -1) => {
    const from = idx < 0 ? (d === 1 ? -1 : list.length) : idx;
    const next = list[Math.max(0, Math.min(list.length - 1, from + d))];
    if (!next) return false as const;
    open(next);
  };
  const remove = () => {
    if (!selected) return false as const;
    const fallback = list[idx + 1] ?? list[idx - 1] ?? null;
    act.mutate({ kind: "archive", id: selected.id });
    setSelectedId(fallback?.id ?? null);
  };

  useShortcuts([
    {
      id: "inbox.next",
      keys: "j",
      context: "list",
      description: "Next notification",
      run: () => move(1),
    },
    {
      id: "inbox.prev",
      keys: "k",
      context: "list",
      description: "Previous notification",
      run: () => move(-1),
    },
    {
      id: "inbox.snooze",
      keys: "h",
      context: "list",
      description: "Snooze notification",
      run: () => {
        if (!selected) return false;
        setSnoozeOpen(true);
      },
    },
    {
      id: "inbox.toggle-read",
      keys: "u",
      context: "list",
      description: "Mark read / unread",
      run: () => {
        if (!selected) return false;
        act.mutate({ kind: "read", id: selected.id, read: !selected.readAt });
      },
    },
    {
      id: "inbox.delete",
      keys: "delete",
      context: "list",
      description: "Delete notification",
      run: remove,
    },
    {
      id: "inbox.delete-bs",
      keys: "backspace",
      context: "list",
      description: "Delete notification",
      run: remove,
    },
    {
      id: "inbox.read-all",
      keys: "alt+u",
      context: "list",
      description: "Mark all as read",
      run: () => act.mutate({ kind: "markAllRead" }),
    },
    {
      id: "inbox.delete-read",
      keys: "shift+backspace",
      context: "list",
      description: "Delete all read",
      run: () => act.mutate({ kind: "archiveAll", filter: "read" }),
    },
  ]);

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState error={q.error} title="Could not load inbox" onRetry={() => void q.refetch()} />
    );
  else if (list.length === 0)
    body = (
      <EmptyState
        title={unreadOnly ? "No unread notifications" : "Inbox zero"}
        description="Mentions, assignments and updates you follow show up here."
      />
    );
  else
    body = (
      <div role="listbox" aria-label="Notifications">
        {list.map((n) => (
          <Row key={n.id} n={n} selected={n.id === selectedId} onSelect={() => open(n)} />
        ))}
      </div>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
        <h1 className="text-sm font-medium">Inbox</h1>
        <span className="text-xs text-muted-foreground">{unreadCount} unread</span>
        <div className="ml-auto flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={unreadOnly}
            className={cn(unreadOnly && "bg-surface-active")}
            onClick={() => setUnreadOnly((v) => !v)}
          >
            Unread only
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={showSnoozed}
            className={cn(showSnoozed && "bg-surface-active")}
            onClick={() => setShowSnoozed((v) => !v)}
          >
            Show snoozed
          </Button>
          <Button variant="ghost" size="sm" onClick={() => act.mutate({ kind: "markAllRead" })}>
            <Check /> Mark all as read
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => act.mutate({ kind: "archiveAll", filter: "read" })}
          >
            <Trash2 /> Delete all read
          </Button>
        </div>
      </header>
      <div className="flex min-h-0 flex-1">
        <div className="w-[380px] shrink-0 overflow-auto border-r border-border">{body}</div>
        <div className="min-w-0 flex-1 overflow-auto">
          {selected ? (
            <div className="flex h-full flex-col">
              <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border px-3">
                <span className="truncate text-sm text-muted-foreground">{selected.title}</span>
                {bundleSummary(selected).count > 1 && (
                  <span
                    data-testid="bundle-detail"
                    className="truncate text-xs text-subtle-foreground"
                  >
                    {bundleSummary(selected).labels.join(" · ")}
                  </span>
                )}
                <div className="ml-auto flex items-center gap-1">
                  <SnoozeMenu
                    open={snoozeOpen}
                    onOpenChange={setSnoozeOpen}
                    onPick={(d) =>
                      act.mutate({ kind: "snooze", id: selected.id, until: d.toISOString() })
                    }
                  >
                    <Button variant="ghost" size="sm" aria-label="Snooze">
                      <Clock /> Snooze
                    </Button>
                  </SnoozeMenu>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={remove}
                    aria-label="Delete notification"
                  >
                    <Trash2 /> Delete
                  </Button>
                </div>
              </div>
              <div className="min-h-0 flex-1 overflow-auto">
                {selected.issueIdentifier ? (
                  <IssueDetailPage
                    key={selected.id}
                    ws={ws}
                    splat={selected.issueIdentifier}
                    embedded
                  />
                ) : (
                  <div className="p-6 text-sm">
                    <p className="mb-1 text-xs tracking-wide text-muted-foreground uppercase">
                      {targetKind(selected)}
                    </p>
                    <p className="mb-3">{selected.subtitle}</p>
                    {bundleSummary(selected).count > 1 && (
                      <ul
                        aria-label="Changes"
                        className="mb-3 list-disc pl-5 text-muted-foreground"
                      >
                        {bundleSummary(selected).labels.map((l) => (
                          <li key={l}>{l}</li>
                        ))}
                      </ul>
                    )}
                    <Link to={selected.url as "/"} className="text-primary hover:underline">
                      Open
                    </Link>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <EmptyState
              title={`${unreadCount} unread notifications`}
              description="Select a notification to see it here."
              action={<BellOff className="size-4 text-subtle-foreground" aria-hidden />}
            />
          )}
        </div>
      </div>
    </div>
  );
}
