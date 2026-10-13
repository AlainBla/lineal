import { Link } from "@tanstack/react-router";
import { ArrowDown, ArrowUp, Columns3, List, Plus } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useActions } from "@/app/commands";
import { teamPaths } from "@/app/nav";
import { Button } from "@/components/ui/button";
import { NotFound } from "@/components/not-found";
import type { Issue } from "@/gql/graphql";
import { cn } from "@/lib/cn";

import { BulkActions, type BulkKind } from "./bulk-actions";
import { openCreateIssue } from "./create-store";
import { useTeamIssues, useTeamMeta } from "./hooks";
import { useIssueActions } from "./issue-actions";
import { useIssueLookup } from "./issue-lookup";
import { IssueBoard } from "./issue-board";
import { IssueList } from "./issue-list";
import {
  DEFAULT_DISPLAY,
  defaultDirection,
  ORDERINGS,
  sortIssues,
  type DisplayOptions,
  type Ordering,
} from "@/features/views/display-options";
import { buildBoardColumns, filterByView, groupByState, type TeamView } from "./model";
import { IssuePickerHost, type PickerRequest } from "./picker-host";
import { useSelection } from "./selection";
import { BoardSkeleton, EmptyState, ErrorState, isNotFound, ListSkeleton } from "./states";
import { useFocusNav } from "./use-focus-nav";
import { useIssueShortcuts } from "./use-issue-shortcuts";

type Layout = "list" | "board";

const VIEW_TITLE: Record<TeamView, string> = {
  active: "Active",
  backlog: "Backlog",
  all: "All issues",
};

const layoutKey = (team: string, view: TeamView) => `lineal-issues-layout:${team}:${view}`;

function readLayout(team: string, view: TeamView): Layout {
  try {
    const v = localStorage.getItem(layoutKey(team, view));
    if (v === "list" || v === "board") return v;
  } catch {
    /* Standard verwenden */
  }
  return view === "active" ? "board" : "list"; // docs/05 §3.1
}

const ORDERING_LABEL: Record<Ordering, string> = {
  priority: "Priority",
  title: "Title",
  status: "Status",
  assignee: "Assignee",
  estimate: "Estimate",
  updatedAt: "Last updated",
  createdAt: "Created",
  dueDate: "Due date",
  linkCount: "Link count",
  timeInStatus: "Time in status",
  manual: "Manual",
};

const orderingKey = (team: string) => `lineal-issues-ordering:${team}`;

/** Gespeicherte Sortierung des Teams; Standard ist die Priorität (docs/05 §2.5, F.13). */
function readOrdering(team: string): Pick<DisplayOptions, "ordering" | "orderDirection"> {
  try {
    const raw = JSON.parse(localStorage.getItem(orderingKey(team)) ?? "null") as {
      ordering?: string;
      orderDirection?: string;
    } | null;
    const ordering = ORDERINGS.find((o) => o === raw?.ordering);
    if (ordering) {
      return {
        ordering,
        orderDirection: raw?.orderDirection === "desc" ? "desc" : "asc",
      };
    }
  } catch {
    /* Standard verwenden */
  }
  return { ordering: "priority", orderDirection: "asc" };
}

/** Team-Issues: Tabs Active/Backlog/All, Liste oder Board (docs/08 §2.1). */
export function TeamIssuesPage({
  ws,
  teamKey,
  view,
}: {
  ws: string;
  teamKey: string;
  view: TeamView;
}) {
  const meta = useTeamMeta(teamKey);
  const issues = useTeamIssues(teamKey);
  const actions = useIssueActions();
  const [layout, setLayoutState] = useState<Layout>(() => readLayout(teamKey, view));
  const [order, setOrderState] = useState(() => readOrdering(teamKey));
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [request, setRequest] = useState<PickerRequest | null>(null);
  const [bulkRequest, setBulkRequest] = useState<BulkKind | null>(null);

  useEffect(() => {
    const prev = document.title;
    document.title = `${teamKey} · ${VIEW_TITLE[view]} · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [teamKey, view]);

  const setLayout = useCallback(
    (l: Layout) => {
      setLayoutState(l);
      try {
        localStorage.setItem(layoutKey(teamKey, view), l);
      } catch {
        /* ignorieren */
      }
    },
    [teamKey, view],
  );

  const setOrder = useCallback(
    (patch: Partial<Pick<DisplayOptions, "ordering" | "orderDirection">>) => {
      setOrderState((prev) => {
        const next = { ...prev, ...patch };
        try {
          localStorage.setItem(orderingKey(teamKey), JSON.stringify(next));
        } catch {
          /* ignorieren */
        }
        return next;
      });
    },
    [teamKey],
  );

  const metas = useMemo(() => [meta.data], [meta.data]);
  const lookup = useIssueLookup(metas);
  const states = useMemo(() => meta.data?.states ?? [], [meta.data]);
  const visible = useMemo(() => filterByView(issues.data?.issues ?? [], view), [issues.data, view]);
  const columns = useMemo(() => buildBoardColumns(states, visible, view), [states, visible, view]);
  // Standardsortierung Priorität; „Manual“ (sortOrder) erlaubt Drag & Drop in der Liste.
  const groups = useMemo(() => {
    const base = groupByState(states, visible, view);
    if (order.ordering === "manual") return base;
    const options: DisplayOptions = { ...DEFAULT_DISPLAY, ...order };
    const ctx = {
      assignee: (id: string | null) => lookup.assigneeName(id),
      statePosition: (id: string) => lookup.state(id)?.position,
    };
    return base.map((g) => ({ ...g, issues: sortIssues(g.issues, options, ctx) }));
  }, [states, visible, view, order, lookup]);

  const grid = useMemo(
    () => (layout === "board" ? columns : groups).map((g) => g.issues.map((i) => i.id)),
    [layout, columns, groups],
  );
  // Fokus auf nicht mehr vorhandenes Issue (gefiltert, verschoben) verwerfen.
  const focused: Issue | null = visible.find((i) => i.id === focusedId) ?? null;
  useFocusNav(layout === "board" ? grid : [grid.flat()], focused?.id ?? null, setFocusedId);
  const selection = useSelection(useMemo(() => grid.flat(), [grid]));
  const selectedIssues = useMemo(
    () => visible.filter((i) => selection.has(i.id)),
    [visible, selection],
  );
  // Mit Auswahl wirken die Picker-Kürzel (S, A, P, L, …) auf alle markierten Issues.
  const openPicker = useCallback(
    (req: PickerRequest) => {
      // Fälligkeit hat keine Bulk-Aktion: immer für das fokussierte Issue.
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
  const closePicker = useCallback(() => setRequest(null), []);

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
    {
      id: "view.toggle-layout",
      title: layout === "board" ? "Switch to list layout" : "Switch to board layout",
      section: "View",
      keys: "mod+b",
      context: "list",
      perform: () => setLayout(layout === "board" ? "list" : "board"),
    },
  ]);

  const paths = teamPaths(ws, teamKey);
  const tabs: Array<[TeamView, string]> = [
    ["active", paths.active],
    ["backlog", paths.backlog],
    ["all", paths.all],
  ];
  const create = (stateId?: string) => openCreateIssue({ teamKey, stateId });

  let body;
  if (issues.isLoading || (meta.isLoading && !meta.data)) {
    body = layout === "board" ? <BoardSkeleton /> : <ListSkeleton />;
  } else if (isNotFound(issues.error) || isNotFound(meta.error)) {
    body = (
      <NotFound
        title="Team not found"
        description={`There is no team ${teamKey} you can access.`}
      />
    );
  } else if (issues.isError || meta.isError) {
    body = (
      <ErrorState
        error={issues.error ?? meta.error}
        onRetry={() => {
          void issues.refetch();
          void meta.refetch();
        }}
      />
    );
  } else if (visible.length === 0 && layout === "list") {
    body = (
      <EmptyState
        title={`No ${view === "all" ? "" : VIEW_TITLE[view].toLowerCase() + " "}issues`}
        description="Issues you create in this team show up here."
        action={
          <Button size="sm" onClick={() => create()}>
            <Plus /> Create issue
          </Button>
        }
      />
    );
  } else if (layout === "board") {
    body = (
      <IssueBoard
        columns={columns}
        ws={ws}
        lookup={lookup}
        focusedId={focused?.id ?? null}
        onFocus={setFocusedId}
        onCreateInColumn={create}
        selection={selection}
        onDrop={({ issue, patch }) => patch && actions.setSortOrder(issue, patch, states)}
      />
    );
  } else {
    body = (
      <IssueList
        groups={groups}
        ws={ws}
        lookup={lookup}
        focusedId={focused?.id ?? null}
        onFocus={setFocusedId}
        onCreateInGroup={create}
        selection={selection}
        onDrop={
          order.ordering === "manual"
            ? ({ issue, patch }) => patch && actions.setSortOrder(issue, patch, states)
            : undefined
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-3">
        <nav aria-label="Issue views" className="flex items-center gap-1">
          {tabs.map(([v, to]) => (
            <Link
              key={v}
              to={to as "/"}
              aria-current={v === view ? "page" : undefined}
              className={cn(
                "rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                v === view && "bg-surface-active text-foreground",
              )}
            >
              {VIEW_TITLE[v]}
            </Link>
          ))}
        </nav>
        <div className="ml-auto flex items-center gap-1">
          {layout === "list" && (
            <>
              <select
                aria-label="Ordering"
                className="h-7 rounded-md border border-border bg-transparent px-1.5 text-xs text-muted-foreground"
                value={order.ordering}
                onChange={(e) => {
                  const ordering = e.target.value as Ordering;
                  setOrder({ ordering, orderDirection: defaultDirection(ordering) });
                }}
              >
                {ORDERINGS.map((o) => (
                  <option key={o} value={o}>
                    {ORDERING_LABEL[o]}
                  </option>
                ))}
              </select>
              <Button
                variant="ghost"
                size="icon"
                aria-label={order.orderDirection === "asc" ? "Ascending order" : "Descending order"}
                disabled={order.ordering === "manual"}
                onClick={() =>
                  setOrder({ orderDirection: order.orderDirection === "asc" ? "desc" : "asc" })
                }
              >
                {order.orderDirection === "asc" ? <ArrowUp /> : <ArrowDown />}
              </Button>
            </>
          )}
          <div
            role="group"
            aria-label="Layout"
            className="flex items-center rounded-md border border-border"
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
          <Button size="sm" onClick={() => create()}>
            <Plus /> New issue
          </Button>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {body}
        {issues.data?.truncated && (
          <p className="px-4 py-3 text-xs text-muted-foreground">
            Showing the first {issues.data.issues.length} issues. Narrow the view to see the rest.
          </p>
        )}
      </div>
      <IssuePickerHost request={request} onClose={closePicker} />
      <BulkActions
        issues={selectedIssues}
        onClear={selection.clear}
        request={bulkRequest}
        onRequest={setBulkRequest}
      />
    </div>
  );
}
