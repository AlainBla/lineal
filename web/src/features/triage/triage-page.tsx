import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Navigate } from "@tanstack/react-router";
import { BellOff, Check, Copy, Forward, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useActions } from "@/app/commands";
import { teamPaths } from "@/app/nav";
import { Button } from "@/components/ui/button";
import { NotFound } from "@/components/not-found";
import type { Issue } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useShortcutContext } from "@/lib/shortcuts";
import { useTeamMeta } from "@/features/issues/hooks";
import { Avatar, PriorityIcon } from "@/features/issues/icons";
import { IssueDetailPage } from "@/features/issues/issue-detail";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { relativeTime } from "@/features/issues/model";
import { IssuePickerHost, type PickerRequest } from "@/features/issues/picker-host";
import { EmptyState, ErrorState, isNotFound, ListSkeleton } from "@/features/issues/states";
import { errorMessage, showToast } from "@/features/issues/toast-store";

import {
  fetchTriageIssues,
  fetchTriageResponsibility,
  triageAccept,
  triageDecline,
  triageDuplicate,
  triageKeys,
  triageSnooze,
  triageUnsnooze,
} from "./queries";
import { TriageDialog, type TriageDialogKind, type TriageSubmit } from "./triage-dialogs";

/** Älteste zuerst (`startedTriageAt`, docs/08 §2.5). */
export function sortTriage(issues: Issue[]): Issue[] {
  const at = (i: Issue) => Date.parse(i.startedTriageAt ?? i.createdAt);
  return [...issues].sort((a, b) => at(a) - at(b) || a.identifier.localeCompare(b.identifier));
}

const isSnoozed = (i: Issue) => !!i.snoozedUntilAt && Date.parse(i.snoozedUntilAt) > Date.now();

/**
 * Triage-Ansicht `/{ws}/team/{KEY}/triage` (docs/08 §2.5, docs/03 §10): links die Triage-Liste,
 * rechts das gewählte Issue mit der Triage-Leiste Accept (1), Decline (2), Duplicate (3), Snooze (H)
 * und Move to team (Ctrl+Shift+M, vom Issue-Detail).
 */
export function TriagePage({ ws, teamKey }: { ws: string; teamKey: string }) {
  const qc = useQueryClient();
  const meta = useTeamMeta(teamKey);
  const teamId = meta.data?.team.id;
  const [showSnoozed, setShowSnoozed] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<TriageDialogKind | null>(null);
  const [picker, setPicker] = useState<PickerRequest | null>(null);
  useShortcutContext("list");

  const list = useQuery({
    queryKey: triageKeys.list(teamId ?? "", showSnoozed),
    enabled: !!teamId && meta.data?.team.triageEnabled === true,
    queryFn: ({ signal }) => fetchTriageIssues(teamId!, showSnoozed, signal),
  });
  const responsibility = useQuery({
    queryKey: ["triage-responsibility", teamId],
    enabled: !!teamId && meta.data?.team.triageEnabled === true,
    queryFn: ({ signal }) => fetchTriageResponsibility(teamId!, signal),
    staleTime: 60_000,
  });
  const metas = useMemo(() => [meta.data], [meta.data]);
  const lookup = useIssueLookup(metas);
  const issues = useMemo(() => sortTriage(list.data ?? []), [list.data]);
  const selected = issues.find((i) => i.id === selectedId) ?? issues[0] ?? null;

  useEffect(() => {
    const prev = document.title;
    document.title = `${teamKey} · Triage · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [teamKey]);

  const invalidate = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["issues"] });
    void qc.invalidateQueries({ queryKey: ["issue"] });
    void qc.invalidateQueries({ queryKey: ["issue-detail"] });
  }, [qc]);

  const run = useMutation({
    mutationFn: async ({ issue, v }: { issue: Issue; v: TriageSubmit }) => {
      switch (v.kind) {
        case "accept":
          return triageAccept(issue.id, {
            stateId: v.stateId,
            priority: v.priority,
            comment: v.comment,
          });
        case "decline":
          return triageDecline(issue.id, { comment: v.comment });
        case "duplicate":
          return triageDuplicate(issue.id, v.duplicateOfId!, v.comment);
        case "snooze":
          return triageSnooze(issue.id, v.snoozedUntilAt!);
      }
    },
    onSettled: invalidate,
  });

  /** Nach der Aktion rückt die Liste zum nächsten Issue vor (Fokus bleibt in der Liste). */
  const submit = async (v: TriageSubmit) => {
    if (!selected) return;
    const index = issues.findIndex((i) => i.id === selected.id);
    const next = issues[index + 1] ?? issues[index - 1] ?? null;
    await run.mutateAsync({ issue: selected, v });
    setSelectedId(next?.id ?? null);
    const verb = {
      accept: "accepted",
      decline: "declined",
      duplicate: "marked as duplicate",
      snooze: "snoozed",
    }[v.kind];
    showToast({ message: `${selected.identifier} ${verb}` });
  };

  const unsnooze = useMutation({
    mutationFn: (issue: Issue) => triageUnsnooze(issue.id),
    onSuccess: invalidate,
    onError: (e) => showToast({ message: errorMessage(e, "Could not unsnooze."), tone: "error" }),
  });

  const open = (kind: TriageDialogKind) => selected && setDialog(kind);
  useActions([
    {
      id: "triage.accept",
      title: "Accept issue",
      section: "Triage",
      keys: "1",
      context: "list",
      enabled: selected !== null && dialog === null,
      perform: () => open("accept"),
    },
    {
      id: "triage.decline",
      title: "Decline issue",
      section: "Triage",
      keys: "2",
      context: "list",
      enabled: selected !== null && dialog === null,
      perform: () => open("decline"),
    },
    {
      id: "triage.duplicate",
      title: "Mark as duplicate…",
      section: "Triage",
      keys: "3",
      context: "list",
      enabled: selected !== null && dialog === null,
      perform: () => open("duplicate"),
    },
    {
      id: "triage.snooze",
      title: "Snooze issue…",
      section: "Triage",
      keys: "h",
      context: "list",
      enabled: selected !== null && dialog === null,
      perform: () => open("snooze"),
    },
  ]);

  const triageOff = meta.data && !meta.data.team.triageEnabled;
  if (triageOff) {
    // docs/08 §1.3: ohne Triage stiller Redirect auf die Standardansicht.
    return <Navigate to={teamPaths(ws, teamKey).all as "/"} replace />;
  }

  let left;
  if (meta.isLoading || list.isLoading) {
    left = <ListSkeleton rows={5} />;
  } else if (isNotFound(meta.error)) {
    return (
      <NotFound
        title="Team not found"
        description={`There is no team ${teamKey} you can access.`}
      />
    );
  } else if (meta.isError || list.isError) {
    left = (
      <ErrorState
        error={meta.error ?? list.error}
        onRetry={() => {
          void meta.refetch();
          void list.refetch();
        }}
      />
    );
  } else if (issues.length === 0) {
    left = (
      <EmptyState
        title="Triage is empty"
        description="New issues from outside the team show up here."
      />
    );
  } else {
    left = (
      <ul aria-label="Triage issues" className="flex flex-col">
        {issues.map((issue) => {
          const creator = lookup.assigneeName(issue.creatorId);
          const snoozed = isSnoozed(issue);
          return (
            <li key={issue.id}>
              <button
                type="button"
                aria-current={issue.id === selected?.id ? "true" : undefined}
                onClick={() => setSelectedId(issue.id)}
                className={cn(
                  "flex w-full flex-col gap-0.5 border-b border-border px-3 py-2 text-left text-sm hover:bg-surface-hover",
                  issue.id === selected?.id && "bg-surface-active",
                )}
              >
                <span className="flex items-center gap-2">
                  <PriorityIcon priority={issue.priority} />
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {issue.identifier}
                  </span>
                  {snoozed && (
                    <span className="inline-flex items-center gap-1 text-xs text-subtle-foreground">
                      <BellOff className="size-3" aria-hidden /> Snoozed
                    </span>
                  )}
                  <span className="ml-auto text-xs text-subtle-foreground">
                    {relativeTime(issue.startedTriageAt ?? issue.createdAt)}
                  </span>
                </span>
                <span className="truncate">{issue.title}</span>
                {creator && (
                  <span className="flex items-center gap-1 text-xs text-muted-foreground">
                    <Avatar name={creator} /> {creator}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    );
  }

  const onTriage = responsibility.data?.currentUser?.name;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3 text-sm">
        <span className="text-muted-foreground">{meta.data?.team.name ?? teamKey}</span>
        <span aria-hidden className="text-muted-foreground">
          ›
        </span>
        <h1 className="font-medium">Triage</h1>
        <span className="text-xs text-muted-foreground" aria-label="Issues in triage">
          {issues.filter((i) => !isSnoozed(i)).length}
        </span>
        {onTriage && <span className="text-xs text-muted-foreground">On triage: {onTriage}</span>}
        <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={showSnoozed}
            onChange={(e) => setShowSnoozed(e.target.checked)}
          />
          Show snoozed
        </label>
      </header>
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Triage list"
          className="w-80 shrink-0 overflow-y-auto border-r border-border"
        >
          {left}
        </nav>
        <section aria-label="Triage detail" className="flex min-w-0 flex-1 flex-col">
          {selected ? (
            <>
              <div
                role="toolbar"
                aria-label="Triage actions"
                className="flex shrink-0 items-center gap-1 border-b border-border px-3 py-1.5"
              >
                <Button size="sm" variant="secondary" onClick={() => open("accept")}>
                  <Check /> Accept <kbd className="ml-1 text-xs text-muted-foreground">1</kbd>
                </Button>
                <Button size="sm" variant="ghost" onClick={() => open("decline")}>
                  <X /> Decline <kbd className="ml-1 text-xs text-muted-foreground">2</kbd>
                </Button>
                <Button size="sm" variant="ghost" onClick={() => open("duplicate")}>
                  <Copy /> Mark as duplicate{" "}
                  <kbd className="ml-1 text-xs text-muted-foreground">3</kbd>
                </Button>
                {isSnoozed(selected) ? (
                  <Button size="sm" variant="ghost" onClick={() => unsnooze.mutate(selected)}>
                    <BellOff /> Unsnooze
                  </Button>
                ) : (
                  <Button size="sm" variant="ghost" onClick={() => open("snooze")}>
                    <BellOff /> Snooze <kbd className="ml-1 text-xs text-muted-foreground">H</kbd>
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setPicker({ kind: "team", issue: selected })}
                >
                  <Forward /> Move to team
                </Button>
              </div>
              <div className="min-h-0 flex-1">
                <IssueDetailPage key={selected.id} ws={ws} splat={selected.identifier} embedded />
              </div>
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
              Select an issue to triage it.
            </div>
          )}
        </section>
      </div>
      <TriageDialog
        kind={dialog}
        issue={selected}
        states={meta.data?.states ?? []}
        onClose={() => setDialog(null)}
        onSubmit={submit}
      />
      <IssuePickerHost request={picker} onClose={() => setPicker(null)} />
    </div>
  );
}
