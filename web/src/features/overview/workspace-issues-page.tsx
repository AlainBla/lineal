import { useEffect, useMemo, useState } from "react";

import { useIssueActions } from "@/features/issues/issue-actions";
import { IssueList } from "@/features/issues/issue-list";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { useTeamMetas, useTeams } from "@/features/issues/hooks";
import { groupByState } from "@/features/issues/model";
import { IssuePickerHost, type PickerRequest } from "@/features/issues/picker-host";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { useIssueShortcuts } from "@/features/issues/use-issue-shortcuts";
import { useFocusNav } from "@/features/issues/use-focus-nav";
import type { Issue } from "@/gql/graphql";

import { useWorkspaceIssues } from "./queries";

/** Teams in Baumreihenfolge (Parent, dann Sub-Teams) mit Einrückungstiefe. */
export function orderTeams<T extends { id: string; parentId: string | null; key: string }>(
  teams: T[],
): Array<{ team: T; depth: number }> {
  const ids = new Set(teams.map((t) => t.id));
  const out: Array<{ team: T; depth: number }> = [];
  const walk = (parent: string | null, depth: number) => {
    for (const t of teams
      .filter((x) => (x.parentId && ids.has(x.parentId) ? x.parentId : null) === parent)
      .sort((a, b) => a.key.localeCompare(b.key))) {
      out.push({ team: t, depth });
      walk(t.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/** Workspace-Issues (`/{ws}/issues`, docs/14 §2): alle sichtbaren Teams, gruppiert nach Team, darunter nach Status. */
export function WorkspaceIssuesPage({ ws }: { ws: string }) {
  const teams = useTeams();
  const issues = useWorkspaceIssues();
  const ordered = useMemo(() => orderTeams(teams.data ?? []), [teams.data]);
  const metas = useTeamMetas(ordered.map((o) => o.team.key));
  const lookup = useIssueLookup(metas);
  const actions = useIssueActions();
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [request, setRequest] = useState<PickerRequest | null>(null);

  useEffect(() => {
    document.title = "Issues · Lineal";
  }, []);

  const sections = useMemo(
    () =>
      ordered.flatMap(({ team, depth }, i) => {
        const meta = metas[i];
        const own = (issues.data?.issues ?? []).filter((x) => x.teamId === team.id);
        if (!meta || own.length === 0) return [];
        return [{ team, depth, groups: groupByState(meta.states, own, "all"), count: own.length }];
      }),
    [ordered, metas, issues.data],
  );
  const all = useMemo(() => sections.flatMap((s) => s.groups.flatMap((g) => g.issues)), [sections]);
  const focused: Issue | null = all.find((i) => i.id === focusedId) ?? null;
  useFocusNav([all.map((i) => i.id)], focused?.id ?? null, setFocusedId);
  useIssueShortcuts({ context: "list", target: focused, open: setRequest, actions });

  let body;
  if (issues.isLoading || teams.isLoading) body = <ListSkeleton />;
  else if (issues.isError || teams.isError)
    body = (
      <ErrorState
        error={issues.error ?? teams.error}
        onRetry={() => {
          void issues.refetch();
          void teams.refetch();
        }}
      />
    );
  else if (sections.length === 0)
    body = <EmptyState title="No issues" description="Issues of all your teams show up here." />;
  else
    body = sections.map((s) => (
      <section key={s.team.id} aria-label={`Team ${s.team.name}`}>
        <h2
          className="flex h-8 items-center gap-2 border-b border-border bg-surface-active/40 px-4 text-sm font-medium"
          style={{ paddingLeft: 16 + s.depth * 16 }}
        >
          <span className="rounded bg-surface-active px-1 text-[10px] text-muted-foreground">
            {s.team.key}
          </span>
          {s.team.name}
          <span className="text-xs text-muted-foreground">{s.count}</span>
        </h2>
        <IssueList
          groups={s.groups}
          ws={ws}
          lookup={lookup}
          focusedId={focused?.id ?? null}
          onFocus={setFocusedId}
        />
      </section>
    ));

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center border-b border-border px-3">
        <h1 className="text-sm font-medium">Issues</h1>
        <span className="ml-2 text-xs text-muted-foreground">All teams</span>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {body}
        {issues.data?.truncated && (
          <p className="px-4 py-3 text-xs text-muted-foreground">
            Showing the first {issues.data.issues.length} issues.
          </p>
        )}
      </div>
      <IssuePickerHost request={request} onClose={() => setRequest(null)} />
    </div>
  );
}
