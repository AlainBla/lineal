import { Link } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import type { Issue } from "@/gql/graphql";

import { useMyAssigned, useMyCreated, useTeamMetas } from "./hooks";
import { StatusIcon } from "./icons";
import { useIssueActions } from "./issue-actions";
import { useIssueLookup } from "./issue-lookup";
import { IssueRow } from "./issue-row";
import { filterAssigned, groupByStateType } from "./model";
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

function MyIssuesList({ tab, ws }: { tab: "assigned" | "created"; ws: string }) {
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

  const flat = useMemo(() => [groups.flatMap((g) => g.issues.map((i) => i.id))], [groups]);
  const focused: Issue | null = issues.find((i) => i.id === focusedId) ?? null;
  useFocusNav(flat, focused?.id ?? null, setFocusedId);
  useIssueShortcuts({ context: "list", target: focused, open: setRequest, actions });

  if (query.isLoading) return <ListSkeleton />;
  if (query.isError) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  if (issues.length === 0) return <EmptyState {...EMPTY[tab]} />;

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
    </>
  );
}

/** My Issues (docs/08 §2.1, docs/05 §3.2): Assigned, Created, Subscribed; Liste nach Status, kein Board. */
export function MyIssuesPage({ ws, tab }: { ws: string; tab: string }) {
  const active: MyIssuesTab = TABS.some((t) => t.id === tab) ? (tab as MyIssuesTab) : "assigned";
  useEffect(() => {
    const prev = document.title;
    document.title = "My issues · Lineal";
    return () => {
      document.title = prev;
    };
  }, []);
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
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {active === "subscribed" ? (
          <EmptyState {...EMPTY.subscribed} />
        ) : (
          <MyIssuesList key={active} tab={active} ws={ws} />
        )}
      </div>
    </div>
  );
}
