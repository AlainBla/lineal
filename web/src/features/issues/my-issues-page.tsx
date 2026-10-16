import { Link } from "@tanstack/react-router";
import { Columns3, List } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useActions } from "@/app/commands";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";
import type { Issue, WorkflowState } from "@/gql/graphql";

import { BulkActions, type BulkKind } from "./bulk-actions";
import { IssueBoard } from "./issue-board";
import { useSelection } from "./selection";
import { useSelectAll } from "./use-select-all";

import { useMyAssigned, useMyCreated, useTeamMetas } from "./hooks";
import { StatusIcon } from "./icons";
import { useIssueActions } from "./issue-actions";
import { useIssueLookup } from "./issue-lookup";
import { IssueRow } from "./issue-row";
import { filterAssigned, groupByStateType, STATE_TYPE_LABEL, type BoardColumn } from "./model";
import { IssuePickerHost, type PickerRequest } from "./picker-host";
import { EmptyState, ErrorState, ListSkeleton } from "./states";
import { useFocusNav } from "./use-focus-nav";
import { useIssueShortcuts } from "./use-issue-shortcuts";

export type MyIssuesTab = "assigned" | "created" | "subscribed";

const TABS: Array<{ id: MyIssuesTab; label: string }> = [
  { id: "assigned", label: "Assigned" },
  { id: "created", label: "Created" },
  { id: "subscribed", label: "Subscribed" },
];

const EMPTY: Record<MyIssuesTab, { title: string; description: string }> = {
  assigned: {
    title: "Nothing assigned to you",
    description: "Issues assigned to you show up here.",
  },
  created: {
    title: "You have not created any issues",
    description: "Issues you create show up here.",
  },
  subscribed: {
    title: "Subscribed issues are not available yet",
    description:
      "The API does not expose issue subscriptions for listing. Subscribe or unsubscribe on an issue.",
  },
};

type Layout = "list" | "board";
const layoutKey = (tab: string) => `lineal-issues-layout:my:${tab}`;

function readLayout(tab: string): Layout {
  try {
    const v = localStorage.getItem(layoutKey(tab));
    if (v === "list" || v === "board") return v;
  } catch {
    /* Standard */
  }
  return "list";
}

/** Board über alle Teams: eine Spalte je Statuskategorie (Teams haben eigene Status). */
const BOARD_TYPES = ["backlog", "unstarted", "started", "completed", "canceled"] as const;
function typeColumns(issues: Issue[]): BoardColumn[] {
  return BOARD_TYPES.map((type, position) => ({
    state: {
      id: `type:${type}`,
      name: STATE_TYPE_LABEL[type],
      type,
      color: "#8a8f98",
      position,
    } as unknown as WorkflowState,
    issues: issues.filter((i) => i.stateType === type),
  }));
}

function MyIssuesList({
  tab,
  ws,
  layout,
}: {
  tab: "assigned" | "created";
  ws: string;
  layout: Layout;
}) {
  const assigned = useMyAssigned();
  const created = useMyCreated();
  const query = tab === "assigned" ? assigned : created;
  const actions = useIssueActions();
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [request, setRequest] = useState<PickerRequest | null>(null);
  const closePicker = useCallback(() => setRequest(null), []);

  const issues = useMemo(() => {
    const all = query.data?.issues ?? [];
    return tab === "assigned" ? filterAssigned(all) : all;
  }, [query.data, tab]);
  const groups = useMemo(() => groupByStateType(issues), [issues]);
  const teamIds = useMemo(() => [...new Set(issues.map((i) => i.teamId))].sort(), [issues]);
  const metas = useTeamMetas(teamIds);
  const lookup = useIssueLookup(metas);

  const columns = useMemo(() => typeColumns(issues), [issues]);
  const grid = useMemo(
    () =>
      layout === "board"
        ? columns.map((c) => c.issues.map((i) => i.id))
        : [groups.flatMap((g) => g.issues.map((i) => i.id))],
    [layout, columns, groups],
  );
  const focused: Issue | null = issues.find((i) => i.id === focusedId) ?? null;
  useFocusNav(grid, focused?.id ?? null, setFocusedId);
  const selection = useSelection(useMemo(() => grid.flat(), [grid]));
  const selectedIssues = useMemo(
    () => issues.filter((i) => selection.has(i.id)),
    [issues, selection],
  );
  const [bulkRequest, setBulkRequest] = useState<BulkKind | null>(null);
  // Mit Auswahl wirken die Picker-Kürzel auf alle markierten Issues (wie in den Team-Listen).
  const openPicker = useCallback(
    (req: PickerRequest) => {
      if (selection.size > 0 && req.kind !== "due" && req.kind !== "parent")
        setBulkRequest(req.kind);
      else setRequest(req);
    },
    [selection.size],
  );
  useIssueShortcuts({
    context: "list",
    target: focused ?? selectedIssues[0] ?? null,
    open: openPicker,
    actions,
  });
  useSelectAll(selection, issues.length > 0);
  useActions([
    {
      id: "issue.select",
      title: "Select issue",
      section: "Issues",
      keys: "x",
      context: "list",
      enabled: focused !== null,
      perform: () => focused && selection.toggle(focused.id),
    },
    {
      id: "issue.clear-selection",
      title: "Clear selection",
      section: "Issues",
      keys: "escape",
      context: "list",
      enabled: selection.size > 0,
      perform: () => selection.clear(),
    },
  ]);

  if (query.isLoading) return <ListSkeleton />;
  if (query.isError) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  if (issues.length === 0) return <EmptyState {...EMPTY[tab]} />;

  if (layout === "board") {
    return (
      <>
        <IssueBoard
          columns={columns}
          ws={ws}
          lookup={lookup}
          focusedId={focusedId}
          onFocus={setFocusedId}
          selection={selection}
          // Spalten sind Statuskategorien über mehrere Teams: Ziehen ändert nichts.
          movesCard={() => false}
          onDrop={() => undefined}
        />
        <IssuePickerHost request={request} onClose={closePicker} />
        <BulkActions
          issues={selectedIssues}
          onClear={selection.clear}
          request={bulkRequest}
          onRequest={setBulkRequest}
        />
      </>
    );
  }
  return (
    <>
      <div role="list" aria-label="Issues">
        {groups.map((g) => (
          <section key={g.type} aria-label={g.label}>
            <header className="flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm">
              <StatusIcon type={g.type} />
              <span className="font-medium">{g.label}</span>
              <span className="text-xs text-muted-foreground">{g.issues.length}</span>
            </header>
            {g.issues.map((issue) => (
              <div role="listitem" key={issue.id}>
                <IssueRow
                  issue={issue}
                  ws={ws}
                  lookup={lookup}
                  focused={focusedId === issue.id}
                  onFocus={setFocusedId}
                  selection={selection}
                />
              </div>
            ))}
          </section>
        ))}
      </div>
      {query.data?.truncated && (
        <p className="px-4 py-3 text-xs text-muted-foreground">
          Showing the first {query.data.issues.length} issues.
        </p>
      )}
      <IssuePickerHost request={request} onClose={closePicker} />
      <BulkActions
        issues={selectedIssues}
        onClear={selection.clear}
        request={bulkRequest}
        onRequest={setBulkRequest}
      />
    </>
  );
}

/** My Issues (docs/08 §2.1, docs/05 §3.2): Assigned, Created, Subscribed; Liste nach Status oder Board (`Ctrl B`), Mehrfachauswahl. */
export function MyIssuesPage({ ws, tab }: { ws: string; tab: string }) {
  const active: MyIssuesTab = TABS.some((t) => t.id === tab) ? (tab as MyIssuesTab) : "assigned";
  useEffect(() => {
    const prev = document.title;
    document.title = "My issues · Lineal";
    return () => {
      document.title = prev;
    };
  }, []);
  const [layout, setLayoutState] = useState<Layout>(() => readLayout(active));
  const setLayout = useCallback(
    (l: Layout) => {
      setLayoutState(l);
      try {
        localStorage.setItem(layoutKey(active), l);
      } catch {
        /* ignorieren */
      }
    },
    [active],
  );
  useActions([
    {
      id: "view.toggle-layout",
      title: layout === "board" ? "Switch to list layout" : "Switch to board layout",
      section: "View",
      keys: "mod+b",
      context: "list",
      enabled: active !== "subscribed",
      perform: () => setLayout(layout === "board" ? "list" : "board"),
    },
  ]);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-3">
        <nav aria-label="My issues tabs" className="flex items-center gap-1">
          {TABS.map((t) => (
            <Link
              key={t.id}
              to={`/${ws}/my-issues/${t.id}` as "/"}
              aria-current={t.id === active ? "page" : undefined}
              className={cn(
                "rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                t.id === active && "bg-surface-active text-foreground",
              )}
            >
              {t.label}
            </Link>
          ))}
        </nav>
        {active !== "subscribed" && (
          <div
            role="group"
            aria-label="Layout"
            className="ml-auto flex items-center rounded-md border border-border"
          >
            <Button
              variant="ghost"
              size="icon"
              aria-label="List layout"
              aria-pressed={layout === "list"}
              className={cn(layout === "list" && "bg-surface-active text-foreground")}
              onClick={() => setLayout("list")}
            >
              <List />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Board layout"
              aria-pressed={layout === "board"}
              className={cn(layout === "board" && "bg-surface-active text-foreground")}
              onClick={() => setLayout("board")}
            >
              <Columns3 />
            </Button>
          </div>
        )}
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {active === "subscribed" ? (
          <EmptyState {...EMPTY.subscribed} />
        ) : (
          <MyIssuesList key={active} tab={active} ws={ws} layout={layout} />
        )}
      </div>
    </div>
  );
}
