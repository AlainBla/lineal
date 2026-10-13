import { Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { openCreateIssue, setCreateContext } from "@/features/issues/create-store";
import { useTeamMetas } from "@/features/issues/hooks";
import { IssueList } from "@/features/issues/issue-list";
import { useIssueLookup } from "@/features/issues/issue-lookup";
import { groupByState, relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Project } from "@/gql/graphql";

import {
  HEALTH_LABEL,
  useCreateMilestone,
  useDeleteMilestone,
  usePostProjectUpdate,
  useProject,
  useProjectUpdates,
  useScopedIssues,
  type Health,
} from "./queries";
import { HealthBadge, PageHeader, ProgressBar, shortDate } from "./ui";

export type ProjectTab = "overview" | "issues" | "updates" | "milestones";
export const PROJECT_TABS: ProjectTab[] = ["overview", "issues", "updates", "milestones"];
export const isProjectTab = (v: string): v is ProjectTab => (PROJECT_TABS as string[]).includes(v);

const TAB_LABEL: Record<ProjectTab, string> = {
  overview: "Overview",
  issues: "Issues",
  updates: "Updates",
  milestones: "Milestones",
};

function Prop({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="text-sm">{children}</dd>
    </div>
  );
}

function UpdateComposer({ projectId }: { projectId: string }) {
  const [body, setBody] = useState("");
  const [health, setHealth] = useState<Health>("onTrack");
  const post = usePostProjectUpdate(projectId);
  return (
    <form
      aria-label="Post update"
      className="flex flex-col gap-2 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!body.trim()) return;
        post.mutate({ body: body.trim(), health }, { onSuccess: () => setBody("") });
      }}
    >
      <div role="radiogroup" aria-label="Health" className="flex gap-1">
        {(Object.keys(HEALTH_LABEL) as Health[]).map((h) => (
          <Button
            key={h}
            type="button"
            variant="ghost"
            size="sm"
            role="radio"
            aria-checked={health === h}
            className={health === h ? "bg-surface-active" : undefined}
            onClick={() => setHealth(h)}
          >
            {HEALTH_LABEL[h]}
          </Button>
        ))}
      </div>
      <textarea
        aria-label="Update text"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        rows={3}
        placeholder="Write a project update…"
        className="w-full resize-y rounded-md border border-input bg-transparent p-2 text-sm outline-none focus-visible:border-ring"
      />
      {post.isError && (
        <p role="alert" className="text-xs text-destructive">
          Could not post update.
        </p>
      )}
      <div className="flex justify-end">
        <Button type="submit" size="sm" disabled={!body.trim() || post.isPending}>
          Post update
        </Button>
      </div>
    </form>
  );
}

function UpdatesFeed({ projectId }: { projectId: string }) {
  const q = useProjectUpdates(projectId);
  if (q.isLoading) return <ListSkeleton rows={3} />;
  if (q.isError) return <ErrorState error={q.error} title="Could not load updates" />;
  if (!q.data?.length) return <p className="text-sm text-muted-foreground">No updates yet.</p>;
  return (
    <ul className="flex flex-col gap-3">
      {q.data.map((u) => (
        <li key={u.id} className="rounded-lg border border-border p-3">
          <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
            <HealthBadge health={u.health} />
            <span>{u.user?.name ?? "Someone"}</span>
            <span>{relativeTime(u.createdAt)}</span>
          </div>
          <RichEditor key={u.id} value={u.body} editable={false} ariaLabel="Update" />
        </li>
      ))}
    </ul>
  );
}

function Milestones({ project }: { project: Project }) {
  const create = useCreateMilestone(project.id);
  const del = useDeleteMilestone(project.id);
  const [name, setName] = useState("");
  const [date, setDate] = useState("");
  return (
    <div className="flex flex-col gap-3 p-4">
      <ul className="divide-y divide-border rounded-lg border border-border">
        {project.milestones.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">No milestones.</li>
        )}
        {project.milestones.map((m) => (
          <li key={m.id} className="flex items-center gap-3 px-3 py-2 text-sm">
            <span className="font-medium">{m.name}</span>
            <span className="text-xs text-muted-foreground">{shortDate(m.targetDate)}</span>
            <ProgressBar value={m.progress} label={`Progress of ${m.name}`} />
            <span className="text-xs text-muted-foreground">
              {m.completedIssueCount} / {m.issueCount}
            </span>
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              aria-label={`Delete milestone ${m.name}`}
              onClick={() => del.mutate(m.id)}
            >
              Delete
            </Button>
          </li>
        ))}
      </ul>
      <form
        aria-label="Add milestone"
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (!name.trim()) return;
          create.mutate(
            { name: name.trim(), targetDate: date || undefined },
            {
              onSuccess: () => {
                setName("");
                setDate("");
              },
            },
          );
        }}
      >
        <Input
          aria-label="Milestone name"
          placeholder="Milestone name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="max-w-xs"
        />
        <Input
          aria-label="Target date"
          type="date"
          value={date}
          onChange={(e) => setDate(e.target.value)}
          className="w-40"
        />
        <Button type="submit" size="sm" disabled={!name.trim()}>
          <Plus /> Add milestone
        </Button>
      </form>
    </div>
  );
}

function ProjectIssues({ ws, project }: { ws: string; project: Project }) {
  const issues = useScopedIssues({ projectId: project.id });
  const metas = useTeamMetas(project.teams.flatMap((t) => (t.key ? [t.key] : [])));
  const lookup = useIssueLookup(metas);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const groups = useMemo(
    () =>
      metas.flatMap((m) =>
        m
          ? groupByState(
              m.states,
              (issues.data?.issues ?? []).filter((i) => i.teamId === m.team.id),
              "all",
            )
          : [],
      ),
    [metas, issues.data],
  );
  const teamKey = project.leadTeam?.key ?? project.teams[0]?.key ?? undefined;
  if (issues.isLoading) return <ListSkeleton />;
  if (issues.isError) return <ErrorState error={issues.error} />;
  return (
    <div>
      <div className="flex justify-end border-b border-border px-3 py-1">
        <Button size="sm" onClick={() => openCreateIssue({ teamKey, projectId: project.id })}>
          <Plus /> New issue
        </Button>
      </div>
      {groups.length === 0 ? (
        <EmptyState title="No issues" description="Issues added to this project show up here." />
      ) : (
        <IssueList
          groups={groups}
          ws={ws}
          lookup={lookup}
          focusedId={focusedId}
          onFocus={setFocusedId}
        />
      )}
    </div>
  );
}

/** Projektseite (docs/08 §2.7): Overview, Issues, Updates, Milestones. */
export function ProjectPage({
  ws,
  projectRef,
  tab,
}: {
  ws: string;
  projectRef: string;
  tab: ProjectTab;
}) {
  const q = useProject(projectRef);
  const p = q.data;
  const updates = useProjectUpdates(p?.id);
  // `c` und der Stift legen Issues im Projekt an (Team und Projekt vorbelegt).
  const ctxTeamKey = p?.leadTeam?.key ?? p?.teams[0]?.key;
  const ctxProjectId = p?.id;
  useEffect(() => {
    if (!ctxProjectId) return;
    setCreateContext({ teamKey: ctxTeamKey, projectId: ctxProjectId });
    return () => setCreateContext(null);
  }, [ctxProjectId, ctxTeamKey]);

  if (q.isLoading) return <ListSkeleton />;
  if (q.isError || !p)
    return (
      <ErrorState error={q.error} title="Could not load project" onRetry={() => void q.refetch()} />
    );

  const latest = updates.data?.[0];
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={p.name}
        docTitle={p.name}
        tabs={PROJECT_TABS.map((t) => ({
          label: TAB_LABEL[t],
          to: `/${ws}/project/${projectRef}/${t}`,
          active: t === tab,
        }))}
        actions={<HealthBadge health={p.health} />}
      />
      <div className="min-h-0 flex-1 overflow-auto">
        {tab === "overview" && (
          <div className="flex max-w-3xl flex-col gap-6 p-4">
            {p.summary && <p className="text-sm text-muted-foreground">{p.summary}</p>}
            <dl className="grid grid-cols-3 gap-4">
              <Prop label="Status">{p.status.name}</Prop>
              <Prop label="Lead">{p.lead?.name ?? "No lead"}</Prop>
              <Prop label="Teams">{p.teams.map((t) => t.key ?? t.name).join(", ") || "–"}</Prop>
              <Prop label="Start">{shortDate(p.startDate)}</Prop>
              <Prop label="Target">{shortDate(p.targetDate)}</Prop>
              <Prop label="Progress">
                <ProgressBar value={p.progress} />
              </Prop>
            </dl>
            <section aria-label="Latest update">
              <h2 className="mb-2 text-xs font-medium text-muted-foreground">Latest update</h2>
              {latest ? (
                <div className="rounded-lg border border-border p-3">
                  <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
                    <HealthBadge health={latest.health} />
                    <span>{relativeTime(latest.createdAt)}</span>
                  </div>
                  <RichEditor
                    key={latest.id}
                    value={latest.body}
                    editable={false}
                    ariaLabel="Latest update"
                  />
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">No updates yet.</p>
              )}
            </section>
            {p.description && (
              <section aria-label="Description">
                <h2 className="mb-2 text-xs font-medium text-muted-foreground">Description</h2>
                <RichEditor
                  key={p.id}
                  value={p.description}
                  editable={false}
                  ariaLabel="Description"
                />
              </section>
            )}
            <section aria-label="Milestones">
              <h2 className="mb-2 text-xs font-medium text-muted-foreground">Milestones</h2>
              {p.milestones.length === 0 ? (
                <p className="text-sm text-muted-foreground">No milestones.</p>
              ) : (
                <ul className="flex flex-col gap-1 text-sm">
                  {p.milestones.map((m) => (
                    <li key={m.id}>
                      {m.name}{" "}
                      <span className="text-xs text-muted-foreground">
                        {shortDate(m.targetDate)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>
        )}
        {tab === "issues" && <ProjectIssues ws={ws} project={p} />}
        {tab === "updates" && (
          <div className="flex max-w-3xl flex-col gap-4 p-4">
            <UpdateComposer projectId={p.id} />
            <UpdatesFeed projectId={p.id} />
          </div>
        )}
        {tab === "milestones" && <Milestones project={p} />}
      </div>
    </div>
  );
}
