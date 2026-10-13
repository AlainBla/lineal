import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  BellOff,
  Check,
  Clock,
  Ellipsis,
  Filter as FilterIcon,
  SlidersHorizontal,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Notification } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useShortcutContext, useShortcuts } from "@/lib/shortcuts";
import { IssueDetailPage } from "@/features/issues/issue-detail";
import { relativeTime } from "@/features/issues/model";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { errorMessage, showToast } from "@/features/issues/toast-store";

import { favoriteObject, favoriteTarget, unsubscribeFrom } from "./inbox-actions";
import {
  buildInbox,
  CATEGORY_LABELS,
  filterCount,
  NO_FILTERS,
  setInboxPrefs,
  useInboxPrefs,
  type InboxFilters,
  type InboxGroup,
  type InboxOrdering,
  type InboxTab,
} from "./inbox-model";
import { bundleSummary, targetKind, typeLabel } from "./notification-types";
import { snoozePresets, useInboxAction, useNotifications } from "./queries";
import { requestRemind, useRemindAction, type RemindTarget } from "./remind";

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

/** Kontextmenü einer Zeile (Rechtsklick oder „…“): Unsubscribe, Favorite, Copy link, Snooze, Delete. */
function RowMenu({
  n,
  open,
  onOpenChange,
  onSnooze,
  onDelete,
}: {
  n: Notification;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onSnooze: (until: Date) => void;
  onDelete: () => void;
}) {
  const fail = (e: unknown) => showToast({ message: errorMessage(e), tone: "error" });
  const unsub = useMutation({
    mutationFn: () => unsubscribeFrom(n),
    onSuccess: () => showToast({ message: "Unsubscribed" }),
    onError: fail,
  });
  const fav = useMutation({
    mutationFn: () => favoriteObject(n),
    onSuccess: () => showToast({ message: "Added to favorites" }),
    onError: fail,
  });
  const hasObject = targetKind(n) !== "system";
  const copyLink = () => {
    const url = /^https?:/.test(n.url) ? n.url : `${window.location.origin}${n.url}`;
    void navigator.clipboard
      ?.writeText(url)
      .then(() => showToast({ message: "Link copied" }))
      .catch(() => showToast({ message: "Could not copy the link", tone: "error" }));
  };
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={`Actions for ${n.title}`}
          className="absolute top-1.5 right-1 hidden size-6 items-center justify-center rounded-md text-muted-foreground group-hover:flex hover:bg-surface-active focus-visible:flex data-[state=open]:flex"
        >
          <Ellipsis className="size-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem disabled={!hasObject} onSelect={() => unsub.mutate()}>
          Unsubscribe
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!favoriteTarget(n)} onSelect={() => fav.mutate()}>
          Favorite
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={copyLink}>Copy link</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Snooze</DropdownMenuLabel>
        {snoozePresets(new Date()).map((p) => (
          <DropdownMenuItem key={p.id} onSelect={() => onSnooze(p.until)}>
            {p.label}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onDelete}>Delete</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Row({
  g,
  selected,
  onSelect,
  onSnooze,
  onDelete,
}: {
  g: InboxGroup;
  selected: boolean;
  onSelect: () => void;
  onSnooze: (until: Date) => void;
  onDelete: () => void;
}) {
  const n = g.latest;
  const unread = g.unread > 0;
  const bundle = bundleSummary(n);
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <div
      className="group relative"
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuOpen(true);
      }}
    >
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
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              unread ? "bg-primary" : "bg-transparent",
            )}
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
          {g.items.length > 1 && (
            <span
              data-testid="group-badge"
              title={`${g.items.length} notifications for this item`}
              className="shrink-0 rounded-full bg-surface-active px-1.5 text-[10px] text-muted-foreground"
            >
              +{g.items.length - 1}
            </span>
          )}
          <span className="ml-auto shrink-0 text-xs text-subtle-foreground group-hover:invisible">
            {relativeTime(n.createdAt)}
          </span>
        </span>
        <span className="truncate pl-3.5 text-xs text-muted-foreground">
          {n.actorName ? `${n.actorName} · ` : ""}
          {n.subtitle}
        </span>
      </button>
      <RowMenu
        n={n}
        open={menuOpen}
        onOpenChange={setMenuOpen}
        onSnooze={onSnooze}
        onDelete={onDelete}
      />
    </div>
  );
}

function FilterDialog({
  open,
  onOpenChange,
  filters,
  onChange,
  actors,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  filters: InboxFilters;
  onChange: (f: InboxFilters) => void;
  actors: Array<{ id: string; name: string }>;
}) {
  const toggle = (key: keyof InboxFilters, v: string) => {
    const cur = filters[key];
    onChange({ ...filters, [key]: cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v] });
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <div className="flex max-h-[70vh] flex-col gap-3 overflow-auto p-4">
          <DialogTitle className="text-sm font-medium">Filter notifications</DialogTitle>
          <DialogDescription className="sr-only">
            Filter by notification type or sender.
          </DialogDescription>
          <fieldset className="flex flex-col gap-1">
            <legend className="mb-1 text-xs text-muted-foreground">Notification type</legend>
            {CATEGORY_LABELS.map(([id, label]) => (
              <label key={id} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={filters.categories.includes(id)}
                  onChange={() => toggle("categories", id)}
                />
                {label}
              </label>
            ))}
          </fieldset>
          {actors.length > 0 && (
            <fieldset className="flex flex-col gap-1">
              <legend className="mb-1 text-xs text-muted-foreground">From</legend>
              {actors.map((a) => (
                <label key={a.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={filters.actorIds.includes(a.id)}
                    onChange={() => toggle("actorIds", a.id)}
                  />
                  {a.name}
                </label>
              ))}
            </fieldset>
          )}
          <Button variant="ghost" size="sm" onClick={() => onChange(NO_FILTERS)}>
            Clear filters
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const selectCls = "h-8 rounded-md border border-input bg-transparent px-2 text-sm";

function DisplayDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
}) {
  const prefs = useInboxPrefs();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-sm font-medium">Display options</DialogTitle>
          <DialogDescription className="sr-only">How the inbox is shown.</DialogDescription>
          <label className="flex items-center justify-between gap-4 text-sm">
            Ordering
            <select
              aria-label="Ordering"
              className={selectCls}
              value={prefs.ordering}
              onChange={(e) => setInboxPrefs({ ordering: e.target.value as InboxOrdering })}
            >
              <option value="newest">Newest</option>
              <option value="oldest">Oldest</option>
              <option value="priority">Priority</option>
            </select>
          </label>
          <label className="flex items-center justify-between gap-4 text-sm">
            Group unreads by
            <select
              aria-label="Group unreads by"
              className={selectCls}
              value={prefs.groupUnreads}
              onChange={(e) =>
                setInboxPrefs({ groupUnreads: e.target.value === "focus" ? "focus" : "none" })
              }
            >
              <option value="none">No grouping</option>
              <option value="focus">Focus</option>
            </select>
          </label>
          <label className="flex items-center justify-between gap-4 text-sm">
            Show unread first
            <input
              type="checkbox"
              aria-label="Show unread first"
              checked={prefs.unreadFirst}
              onChange={(e) => setInboxPrefs({ unreadFirst: e.target.checked })}
            />
          </label>
          <label className="flex items-center justify-between gap-4 text-sm">
            Enable priority inbox
            <input
              type="checkbox"
              aria-label="Enable priority inbox"
              checked={prefs.priorityInbox}
              onChange={(e) => setInboxPrefs({ priorityInbox: e.target.checked })}
            />
          </label>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function remindTargetOf(n: Notification | null): RemindTarget | null {
  if (!n) return null;
  if (n.documentId) return { targetType: "document", target: n.documentId, label: n.title };
  if (n.projectId && targetKind(n) === "project")
    return { targetType: "project", target: n.projectId, label: n.title };
  if (n.initiativeId) return { targetType: "initiative", target: n.initiativeId, label: n.title };
  return null;
}

/** Inbox (docs/08 §2.9, F13): Liste links, Bezugsobjekt rechts; Tastatur J/K, U, Entf, Alt+U. */
export function InboxPage({ ws }: { ws: string }) {
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [showSnoozed, setShowSnoozed] = useState(false);
  const [filters, setFilters] = useState<InboxFilters>(NO_FILTERS);
  const [tab, setTab] = useState<InboxTab>("priority");
  const [filterOpen, setFilterOpen] = useState(false);
  const [displayOpen, setDisplayOpen] = useState(false);
  const [confirmAll, setConfirmAll] = useState(false);
  const prefs = useInboxPrefs();
  const q = useNotifications({ unreadOnly, showSnoozed });
  const act = useInboxAction();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snoozeOpen, setSnoozeOpen] = useState(false);
  const raw = useMemo(() => q.data ?? [], [q.data]);
  const view = useMemo(() => buildInbox(raw, prefs, filters, tab), [raw, prefs, filters, tab]);
  const groups = view.groups;
  // Die Auswahl führt über die neueste Notification der Gruppe (Server-Aktionen wirken auf die Gruppe).
  const idx = groups.findIndex((g) => g.items.some((n) => n.id === selectedId));
  const selectedGroup = idx >= 0 ? groups[idx]! : null;
  const selected = selectedGroup?.latest ?? null;
  const unreadCount = groups.filter((g) => g.unread > 0).length;
  const actors = useMemo(() => {
    const m = new Map<string, string>();
    for (const n of raw) if (n.actorId && n.actorName) m.set(n.actorId, n.actorName);
    return [...m.entries()].map(([id, name]) => ({ id, name }));
  }, [raw]);
  useShortcutContext("list");
  useRemindAction(remindTargetOf(selected), "list");

  useEffect(() => {
    document.title = "Inbox · Lineal";
  }, []);

  // Öffnen einer Gruppe markiert sie als gelesen (der Server gilt für die ganze Gruppe).
  const open = (g: InboxGroup) => {
    setSelectedId(g.latest.id);
    if (g.unread > 0) act.mutate({ kind: "read", id: g.latest.id, read: true });
  };
  const move = (d: 1 | -1) => {
    const from = idx < 0 ? (d === 1 ? -1 : groups.length) : idx;
    const next = groups[Math.max(0, Math.min(groups.length - 1, from + d))];
    if (!next) return false as const;
    open(next);
  };
  const removeGroup = (g: InboxGroup) => {
    const i = groups.indexOf(g);
    const fallback = groups[i + 1] ?? groups[i - 1] ?? null;
    act.mutate({ kind: "archive", id: g.latest.id });
    if (selectedGroup === g) setSelectedId(fallback?.latest.id ?? null);
  };
  const remove = () => {
    if (!selectedGroup) return false as const;
    removeGroup(selectedGroup);
  };
  const deleteAll = (filter: "all" | "read" | "completed") => {
    act.mutate({ kind: "archiveAll", filter });
    setSelectedId(null);
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
        if (!selectedGroup) return false;
        act.mutate({ kind: "read", id: selectedGroup.latest.id, read: selectedGroup.unread === 0 });
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

  const renderRow = (g: InboxGroup) => (
    <Row
      key={g.key}
      g={g}
      selected={g.items.some((n) => n.id === selectedId)}
      onSelect={() => open(g)}
      onSnooze={(d) => act.mutate({ kind: "snooze", id: g.latest.id, until: d.toISOString() })}
      onDelete={() => removeGroup(g)}
    />
  );

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState error={q.error} title="Could not load inbox" onRetry={() => void q.refetch()} />
    );
  else if (groups.length === 0)
    body = (
      <EmptyState
        title={
          filterCount(filters) > 0
            ? "No notifications match the filters"
            : unreadOnly
              ? "No unread notifications"
              : prefs.priorityInbox && tab === "priority"
                ? "No priority notifications"
                : "Inbox zero"
        }
        description="Mentions, assignments and updates you follow show up here."
      />
    );
  else if (prefs.groupUnreads === "focus") {
    const unread = groups.filter((g) => g.unread > 0);
    const rest = groups.filter((g) => g.unread === 0);
    body = (
      <div role="listbox" aria-label="Notifications">
        {unread.length > 0 && (
          <div role="group" aria-label="Unread">
            <h2 className="px-3 py-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
              Unread
            </h2>
            {unread.map(renderRow)}
          </div>
        )}
        {rest.length > 0 && (
          <div role="group" aria-label="Earlier">
            {unread.length > 0 && (
              <h2 className="px-3 py-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                Earlier
              </h2>
            )}
            {rest.map(renderRow)}
          </div>
        )}
      </div>
    );
  } else
    body = (
      <div role="listbox" aria-label="Notifications">
        {groups.map(renderRow)}
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
          <Button
            variant="ghost"
            size="sm"
            className={cn(filterCount(filters) > 0 && "bg-surface-active")}
            onClick={() => setFilterOpen(true)}
          >
            <FilterIcon /> Filter{filterCount(filters) > 0 ? ` (${filterCount(filters)})` : ""}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setDisplayOpen(true)}>
            <SlidersHorizontal /> Display
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
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" aria-label="Notification actions">
                <Ellipsis />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => act.mutate({ kind: "markAllRead" })}>
                Mark all as read
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setConfirmAll(true)}>Delete all</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => deleteAll("read")}>
                Delete all read
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => deleteAll("completed")}>
                Delete all completed
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem asChild>
                <Link to={`/${ws}/settings/account/notifications` as "/"}>Go to settings</Link>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </header>
      {prefs.priorityInbox && (
        <div
          role="tablist"
          aria-label="Inbox tabs"
          className="flex gap-1 border-b border-border px-3"
        >
          {(
            [
              ["priority", "Priority", view.priorityCount],
              ["other", "Other", view.otherCount],
            ] as const
          ).map(([id, label, count]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              onClick={() => {
                setTab(id);
                setSelectedId(null);
              }}
              className={cn(
                "border-b-2 px-2 py-1.5 text-sm",
                tab === id
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground",
              )}
            >
              {label} <span className="text-xs text-subtle-foreground">{count}</span>
            </button>
          ))}
        </div>
      )}
      <FilterDialog
        open={filterOpen}
        onOpenChange={setFilterOpen}
        filters={filters}
        onChange={setFilters}
        actors={actors}
      />
      <DisplayDialog open={displayOpen} onOpenChange={setDisplayOpen} />
      <Dialog open={confirmAll} onOpenChange={setConfirmAll}>
        <DialogContent className="max-w-sm">
          <div className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-sm font-medium">Delete all notifications?</DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground">
              All notifications in your inbox are removed. This cannot be undone.
            </DialogDescription>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setConfirmAll(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                onClick={() => {
                  setConfirmAll(false);
                  deleteAll("all");
                }}
              >
                Delete all
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
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
                  {(selected.issueIdentifier || remindTargetOf(selected)) && (
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label="Remind me"
                      onClick={() => {
                        const t: RemindTarget | null = selected.issueIdentifier
                          ? {
                              targetType: "issue",
                              target: selected.issueIdentifier,
                              label: selected.issueIdentifier,
                            }
                          : remindTargetOf(selected);
                        if (t) requestRemind(t);
                      }}
                    >
                      <Clock /> Remind me
                    </Button>
                  )}
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
