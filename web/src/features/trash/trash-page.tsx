import { useState } from "react";

import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { relativeTime } from "@/features/issues/model";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { PageHeader } from "@/features/planning/ui";

import { useRestore, useTrashedInitiatives, useTrashedIssues, useTrashedProjects } from "./queries";

export type TrashTab = "issues" | "projects" | "initiatives";
export const TRASH_TABS: TrashTab[] = ["issues", "projects", "initiatives"];
export const isTrashTab = (v: string): v is TrashTab => (TRASH_TABS as string[]).includes(v);

const TAB_LABEL: Record<TrashTab, string> = {
  issues: "Issues",
  projects: "Projects",
  initiatives: "Initiatives",
};

interface Row {
  id: string;
  label: string;
  title: string;
  deletedAt: string;
}

function RestoreList({
  kind,
  rows,
  noun,
}: {
  kind: "issue" | "project" | "initiative";
  rows: Row[];
  noun: string;
}) {
  const restore = useRestore(kind);
  const [pending, setPending] = useState<string | null>(null);
  if (rows.length === 0)
    return (
      <EmptyState
        title={`No deleted ${noun}s`}
        description={`Deleted ${noun}s stay here for 30 days and can be restored.`}
      />
    );
  return (
    <div className="flex flex-col gap-2 p-4">
      {restore.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(restore.error)}
        </p>
      )}
      <ul className="divide-y divide-border rounded-lg border border-border">
        {rows.map((r) => (
          <li key={r.id} className="flex items-center gap-3 px-3 py-1.5 text-sm">
            <span className="w-20 shrink-0 text-xs text-muted-foreground">{r.label}</span>
            <span className="truncate">{r.title}</span>
            <span className="ml-auto text-xs text-muted-foreground">
              Deleted {relativeTime(r.deletedAt)}
            </span>
            <Button
              variant="outline"
              size="sm"
              aria-label={`Restore ${r.title}`}
              disabled={restore.isPending && pending === r.id}
              onClick={() => {
                setPending(r.id);
                restore.mutate(r.id);
              }}
            >
              Restore
            </Button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function IssuesTab({ teamKey }: { teamKey?: string }) {
  const q = useTrashedIssues(teamKey);
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load deleted issues" />;
  return (
    <RestoreList
      kind="issue"
      noun="issue"
      rows={(q.data ?? []).map((i) => ({
        id: i.id,
        label: i.identifier,
        title: i.title,
        deletedAt: i.updatedAt,
      }))}
    />
  );
}

function ProjectsTab() {
  const q = useTrashedProjects();
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load deleted projects" />;
  return (
    <RestoreList
      kind="project"
      noun="project"
      rows={(q.data ?? []).map((p) => ({
        id: p.id,
        label: p.leadTeam?.key ?? "",
        title: p.name,
        deletedAt: p.updatedAt,
      }))}
    />
  );
}

function InitiativesTab() {
  const q = useTrashedInitiatives();
  if (q.isLoading) return <ListSkeleton rows={4} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load deleted initiatives" />;
  return (
    <RestoreList
      kind="initiative"
      noun="initiative"
      rows={(q.data ?? []).map((i) => ({
        id: i.id,
        label: i.identifier,
        title: i.name,
        deletedAt: i.updatedAt,
      }))}
    />
  );
}

/** „Recently deleted“ (docs/03 §9, docs/04 §2.12): workspaceweit mit Tabs, je Team nur Issues. */
export function TrashPage({
  ws,
  tab = "issues",
  teamKey,
}: {
  ws: string;
  tab?: TrashTab;
  teamKey?: string;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Recently deleted"
        docTitle={teamKey ? `${teamKey} · Recently deleted` : "Recently deleted"}
        tabs={
          teamKey
            ? undefined
            : TRASH_TABS.map((t) => ({
                label: TAB_LABEL[t],
                to: `/${ws}/trash/${t}`,
                active: t === tab,
              }))
        }
      />
      <div className="min-h-0 flex-1 overflow-auto">
        {(teamKey || tab === "issues") && <IssuesTab teamKey={teamKey} />}
        {!teamKey && tab === "projects" && <ProjectsTab />}
        {!teamKey && tab === "initiatives" && <InitiativesTab />}
      </div>
    </div>
  );
}
