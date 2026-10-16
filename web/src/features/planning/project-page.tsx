import { PanelRight, Plus } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CommentsSection } from "@/features/comments/comments-section";
import { DocumentsSection } from "@/features/docs/documents-section";
import { errorText } from "@/features/issues/error-text";
import { openCreateIssue, setCreateContext } from "@/features/issues/create-store";
import { relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Project } from "@/gql/graphql";

import { DateField, TextField } from "./fields";
import { ProjectActions, ProjectDetails } from "./project-details";
import { ProjectPanel } from "./project-panel";
import {
  HEALTH_LABEL,
  useCreateMilestone,
  useDeleteMilestone,
  useUpdateMilestone,
  usePostProjectUpdate,
  useProject,
  useProjectUpdates,
  type Health,
} from "./queries";
import { useRemindAction } from "@/features/inbox/remind";
import { IssueView } from "@/features/views/issue-view";
import { cn } from "@/lib/cn";
import { HealthBadge, PageHeader, ProgressBar, shortDate } from "./ui";

export type ProjectTab = "overview" | "issues" | "updates" | "milestones" | "resources";
export const PROJECT_TABS: ProjectTab[] = [
  "overview",
  "issues",
  "updates",
  "milestones",
  "resources",
];
export const isProjectTab = (v: string): v is ProjectTab => (PROJECT_TABS as string[]).includes(v);

const TAB_LABEL: Record<ProjectTab, string> = {
  overview: "Overview",
  issues: "Issues",
  updates: "Updates",
  milestones: "Milestones",
  resources: "Resources",
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
  // Ref-Lock: ein Doppelklick vor dem Re-Render legt kein zweites Update an.
  const busy = useRef(false);
  return (
    <form
      aria-label="Post update"
      className="flex flex-col gap-2 rounded-lg border border-border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!body.trim() || busy.current) return;
        busy.current = true;
        post.mutate(
          { body: body.trim(), health },
          { onSuccess: () => setBody(""), onSettled: () => (busy.current = false) },
        );
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
          {errorText(post.error)}
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
          <div className="mt-3">
            <CommentsSection
              target={{ projectUpdateId: u.id }}
              title={`Comments on update ${relativeTime(u.createdAt)}`}
            />
          </div>
        </li>
      ))}
    </ul>
  );
}

function Milestones({ project }: { project: Project }) {
  const create = useCreateMilestone(project.id);
  const del = useDeleteMilestone(project.id);
  const upd = useUpdateMilestone(project.id);
  const [name, setName] = useState("");
  const [date, setDate] = useState("");
  return (
    <div className="flex flex-col gap-3 p-4">
      <ul className="divide-y divide-border rounded-lg border border-border">
        {project.milestones.length === 0 && (
          <li className="px-3 py-2 text-sm text-muted-foreground">No milestones.</li>
        )}
        {project.milestones.map((m) => (
          <li key={m.id} className="flex flex-wrap items-end gap-3 px-3 py-2 text-sm">
            <div className="w-56">
              <TextField
                label={`Milestone name ${m.name}`}
                value={m.name}
                required
                maxLength={255}
                onCommit={(name) => upd.mutate({ id: m.id, input: { name } })}
              />
            </div>
            <DateField
              label={`Target date of ${m.name}`}
              date={m.targetDate}
              dayOnly
              onChange={(targetDate) => upd.mutate({ id: m.id, input: { targetDate } })}
            />
            <ProgressBar value={m.progress} label={`Progress of ${m.name}`} />
            <span className="text-xs text-muted-foreground">
              {m.completedIssueCount} / {m.issueCount}
            </span>
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto"
              aria-label={`Delete milestone ${m.name}`}
              onClick={() => {
                if (
                  window.confirm(
                    `Delete milestone "${m.name}"? Its issues stay in the project without a milestone.`,
                  )
                )
                  del.mutate(m.id);
              }}
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
        <Button type="submit" size="sm" disabled={!name.trim() || create.isPending}>
          <Plus /> Add milestone
        </Button>
      </form>
      {(create.isError || del.isError || upd.isError) && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(create.error ?? del.error ?? upd.error)}
        </p>
      )}
    </div>
  );
}

const PANEL_KEY = "lineal.project.panel";

/** Seitenpanel ein/aus, pro Browser gemerkt (Standard: ein). */
function usePanelOpen(): [boolean, (v: boolean) => void] {
  const [open, setOpen] = useState(() => {
    try {
      return window.localStorage.getItem(PANEL_KEY) !== "0";
    } catch {
      return true;
    }
  });
  return [
    open,
    (v) => {
      setOpen(v);
      try {
        window.localStorage.setItem(PANEL_KEY, v ? "1" : "0");
      } catch {
        /* ohne Speicher bleibt es beim Sitzungswert */
      }
    },
  ];
}

function ProjectIssues({
  ws,
  project,
  toolbarSlot,
  panelOpen,
  setPanelOpen,
}: {
  ws: string;
  project: Project;
  toolbarSlot: HTMLElement | null;
  panelOpen: boolean;
  setPanelOpen: (v: boolean) => void;
}) {
  const teamKey = project.leadTeam?.key ?? project.teams[0]?.key ?? undefined;
  const scope = useMemo(() => ({ project: { id: { in: [project.id] } } }), [project.id]);
  const prefsTarget = useMemo(
    () => ({ viewType: "projectIssues", projectId: project.id }),
    [project.id],
  );
  const emptyGroups = useMemo(
    () => ({
      milestone: [
        ...project.milestones.map((m) => ({ key: m.id, label: m.name })),
        { key: "none", label: "No milestone" },
      ],
    }),
    [project.milestones],
  );
  const dates = useMemo(
    () => new Map(project.milestones.map((m) => [m.id, m.targetDate] as const)),
    [project.milestones],
  );
  return (
    <div className="flex h-full min-h-0">
      <div className="min-w-0 flex-1">
        <IssueView
          ws={ws}
          title="Issues"
          scope={scope}
          prefsTarget={prefsTarget}
          embedded
          toolbarSlot={toolbarSlot}
          emptyGroups={emptyGroups}
          trailingControls={
            <Button
              variant="ghost"
              size="icon"
              aria-label={panelOpen ? "Hide side panel" : "Show side panel"}
              aria-pressed={panelOpen}
              title="Toggle side panel"
              className={cn(panelOpen && "bg-surface-active")}
              onClick={() => setPanelOpen(!panelOpen)}
            >
              <PanelRight />
            </Button>
          }
          renderGroupExtra={(g) => {
            const milestoneId = dates.has(g.key) ? g.key : undefined;
            const date = dates.get(g.key);
            return (
              <>
                {date && (
                  <span className="text-xs font-normal text-muted-foreground">
                    {shortDate(date).replace(/ \d{4}$/, "")}
                  </span>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`New issue in ${g.label}`}
                  onClick={() => openCreateIssue({ teamKey, projectId: project.id, milestoneId })}
                >
                  <Plus />
                </Button>
              </>
            );
          }}
        />
      </div>
      {panelOpen && <ProjectPanel project={project} ws={ws} />}
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
  const [toolbarSlot, setToolbarSlot] = useState<HTMLElement | null>(null);
  const [panelOpen, setPanelOpen] = usePanelOpen();
  // `c` und der Stift legen Issues im Projekt an (Team und Projekt vorbelegt).
  const ctxTeamKey = p?.leadTeam?.key ?? p?.teams[0]?.key ?? undefined;
  const ctxProjectId = p?.id;
  useEffect(() => {
    if (!ctxProjectId) return;
    setCreateContext({ teamKey: ctxTeamKey, projectId: ctxProjectId });
    return () => setCreateContext(null);
  }, [ctxProjectId, ctxTeamKey]);

  useRemindAction(p ? { targetType: "project", target: p.id, label: p.name } : null);

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
        actions={
          <>
            <HealthBadge health={p.health} />
            <ProjectActions project={p} ws={ws} />
          </>
        }
      />
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
        <nav aria-label="Sections" className="flex items-center gap-1">
          {PROJECT_TABS.map((t) => (
            <Link
              key={t}
              to={`/${ws}/project/${projectRef}/${t}` as "/"}
              aria-current={t === tab ? "page" : undefined}
              className={cn(
                "rounded-full border border-border px-3 py-0.5 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                t === tab && "bg-surface-active text-foreground",
              )}
            >
              {TAB_LABEL[t]}
            </Link>
          ))}
        </nav>
        <div ref={setToolbarSlot} className="ml-auto flex items-center gap-1" />
      </div>
      <div className={cn("min-h-0 flex-1", tab === "issues" ? "overflow-hidden" : "overflow-auto")}>
        {tab === "overview" && (
          <div className="flex max-w-3xl flex-col gap-6 p-4">
            <ProjectDetails project={p} ws={ws} />
            <Prop label="Progress">
              <ProgressBar value={p.progress} />
            </Prop>
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
            <CommentsSection target={{ projectId: p.id }} title="Project comments" />
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
        {tab === "issues" && (
          <ProjectIssues
            ws={ws}
            project={p}
            toolbarSlot={toolbarSlot}
            panelOpen={panelOpen}
            setPanelOpen={setPanelOpen}
          />
        )}
        {tab === "updates" && (
          <div className="flex max-w-3xl flex-col gap-4 p-4">
            <UpdateComposer projectId={p.id} />
            <UpdatesFeed projectId={p.id} />
          </div>
        )}
        {tab === "milestones" && <Milestones project={p} />}
        {tab === "resources" && (
          <DocumentsSection
            ws={ws}
            scope={{ projectId: p.id }}
            teamId={p.leadTeam?.id ?? p.teams[0]?.id}
          />
        )}
      </div>
    </div>
  );
}
