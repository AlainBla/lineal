/** Detailseite eines Meilensteins (docs/04 §3, docs/08): Arbeit, Kontext und Verlauf an einem Ort. */
import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { Check, ChevronDown, ChevronRight, Diamond, MoreHorizontal, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { CommentsSection } from "@/features/comments/comments-section";
import { requestRemind, useRemindAction } from "@/features/inbox/remind";
import { openCreateIssue } from "@/features/issues/create-store";
import { errorText } from "@/features/issues/error-text";
import { StatusIcon } from "@/features/issues/icons";
import { useIssueLookup, type IssueLookup } from "@/features/issues/issue-lookup";
import { IssueRow } from "@/features/issues/issue-row";
import { relativeTime } from "@/features/issues/model";
import { RichEditor } from "@/features/issues/rich-editor";
import { ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Issue, Project, ProjectMilestone } from "@/gql/graphql";
import { useViewIssues } from "@/features/views/queries";
import { selectClass } from "@/features/settings/ui";
import { useFilterSources } from "@/features/views/use-filter-sources";
import { cn } from "@/lib/cn";

import { MilestoneDescription } from "./description-editor";
import { DateField, TextField } from "./fields";
import {
  useCompleteMilestone,
  useConvertMilestone,
  useDeleteMilestone,
  useMilestoneSnapshot,
  useProject,
  useProjectUpdates,
  useReopenMilestone,
  useUpdateMilestone,
} from "./queries";
import { ResourcesSection } from "./resources";
import {
  HealthBadge,
  mentionsMilestone,
  MILESTONE_COLOR,
  MilestoneBar,
  MilestoneStatusBadge,
  PageHeader,
  shiftLabel,
  shortDate,
  targetShift,
} from "./ui";

/** Reihenfolge der Statusgruppen: laufende Arbeit zuerst, Abgeschlossenes zuletzt. */
const STATE_ORDER = ["started", "unstarted", "backlog", "triage", "completed", "canceled"];
const FOLDED = new Set(["completed", "canceled", "duplicate"]);

interface StateGroup {
  key: string;
  name: string;
  type: string;
  stateId: string;
  issues: Issue[];
}

export function groupByState(issues: Issue[]): StateGroup[] {
  const groups = new Map<string, StateGroup>();
  for (const i of issues) {
    const g = groups.get(i.stateName);
    if (g) g.issues.push(i);
    else
      groups.set(i.stateName, {
        key: i.stateName,
        name: i.stateName,
        type: i.stateType,
        stateId: i.stateId,
        issues: [i],
      });
  }
  const rank = (t: string) => {
    const r = STATE_ORDER.indexOf(t);
    return r < 0 ? STATE_ORDER.length : r;
  };
  return [...groups.values()].sort((a, b) => rank(a.type) - rank(b.type));
}

function Group({
  group,
  ws,
  lookup,
  added,
}: {
  group: StateGroup;
  ws: string;
  lookup: IssueLookup;
  added: ReadonlySet<string>;
}) {
  const [open, setOpen] = useState(!FOLDED.has(group.type));
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        className="flex h-8 w-full items-center gap-2 rounded-md bg-surface px-2 text-sm font-medium"
        onClick={() => setOpen((o) => !o)}
      >
        <StatusIcon type={group.type} color={lookup.state(group.stateId)?.color} />
        {group.name}
        <span className="font-normal text-muted-foreground tabular-nums">
          {group.issues.length}
        </span>
        {open ? (
          <ChevronDown className="ml-auto size-3.5 text-muted-foreground" />
        ) : (
          <ChevronRight className="ml-auto size-3.5 text-muted-foreground" />
        )}
      </button>
      {open &&
        group.issues.map((i) => (
          <IssueRow
            key={i.id}
            issue={i}
            ws={ws}
            lookup={lookup}
            focused={false}
            onFocus={noop}
            badge={
              added.has(i.id) ? (
                <span className="shrink-0 rounded-full border border-amber-500/40 px-1.5 text-xs text-amber-500">
                  added
                </span>
              ) : undefined
            }
          />
        ))}
    </div>
  );
}

const noop = () => {};

/**
 * Zähler und Status liegen am Projekt-Query. Ändert sich die Issue-Liste (neues Issue, Statuswechsel,
 * Sync), wird es neu geladen, damit Kopf und Liste nicht auseinanderlaufen.
 */
function useRefreshCounts(issues: Issue[] | undefined) {
  const qc = useQueryClient();
  const seen = useRef<string | null>(null);
  const signature = issues?.map((i) => `${i.id}:${i.stateType}`).join(",");
  useEffect(() => {
    if (signature === undefined) return;
    if (seen.current !== null && seen.current !== signature) {
      void qc.invalidateQueries({ queryKey: ["planning", "project"] });
    }
    seen.current = signature;
  }, [signature, qc]);
}

function MilestoneIssues({
  ws,
  project,
  milestone,
}: {
  ws: string;
  project: Project;
  milestone: ProjectMilestone;
}) {
  const q = useViewIssues({ filter: { projectMilestone: { id: { in: [milestone.id] } } } });
  const { metas } = useFilterSources();
  const lookup = useIssueLookup(metas);
  const groups = useMemo(() => groupByState(q.data?.issues ?? []), [q.data]);
  useRefreshCounts(q.data?.issues);
  const teamKey = project.leadTeam?.key ?? project.teams[0]?.key ?? undefined;
  const added = useMemo(() => new Set(milestone.addedIssueIds), [milestone.addedIssueIds]);
  return (
    <section aria-label="Issues" className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium">Issues</h2>
        <Button
          variant="outline"
          size="sm"
          onClick={() =>
            openCreateIssue({ teamKey, projectId: project.id, milestoneId: milestone.id })
          }
        >
          <Plus /> New issue
        </Button>
      </div>
      {q.isLoading && <ListSkeleton rows={3} />}
      {q.isError && (
        <ErrorState
          title="Could not load issues"
          error={q.error}
          onRetry={() => void q.refetch()}
        />
      )}
      {q.isSuccess && groups.length === 0 && (
        <p className="text-sm text-muted-foreground">No issues in this milestone yet.</p>
      )}
      {groups.map((g) => (
        <Group key={g.key} group={g} ws={ws} lookup={lookup} added={added} />
      ))}
      {q.data?.truncated && (
        <p className="text-xs text-muted-foreground">Showing the first 250 issues.</p>
      )}
    </section>
  );
}

function Summary({ milestone: m }: { milestone: ProjectMilestone }) {
  const open = Math.max(0, m.issueCount - m.completedIssueCount - m.startedIssueCount);
  return (
    <div role="group" aria-label={`Progress of ${m.name}`} className="flex flex-col gap-1.5">
      <div className="flex flex-wrap gap-x-3 text-sm text-muted-foreground tabular-nums">
        <span className="font-medium text-foreground">
          {m.completedIssueCount} of {m.issueCount} done
        </span>
        <span>{m.startedIssueCount} in progress</span>
        <span>{open} not started</span>
        {m.addedIssueIds.length > 0 && (
          <span className="text-amber-500">{m.addedIssueIds.length} added since start</span>
        )}
      </div>
      <MilestoneBar
        done={m.completedIssueCount}
        started={m.startedIssueCount}
        total={m.issueCount}
      />
    </div>
  );
}

/** Die beim Abschließen eingefrorene Issue-Liste. */
function FrozenIssues({ ws, project, id }: { ws: string; project: Project; id: string }) {
  const q = useMilestoneSnapshot(project.id, id, true);
  return (
    <section aria-label="Issues at completion" className="flex flex-col gap-2">
      <h2 className="text-sm font-medium">Issues at completion</h2>
      {q.isLoading && <ListSkeleton rows={3} />}
      {q.isError && <ErrorState title="Could not load issues" error={q.error} />}
      <ul className="flex flex-col">
        {q.data?.map((i) => (
          <li key={i.id} className="flex h-8 items-center gap-2 px-2 text-sm">
            <StatusIcon type={i.stateType} />
            <Link
              to={`/${ws}/issue/${i.identifier}` as "/"}
              className="w-16 shrink-0 text-xs text-muted-foreground hover:underline"
            >
              {i.identifier}
            </Link>
            <span className="min-w-0 flex-1 truncate">{i.title}</span>
            <span className="shrink-0 text-xs text-muted-foreground">{i.stateName}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Statusberichte des Projekts, die diesen Meilenstein erwähnen; ohne Treffer kein Abschnitt. */
function MilestoneUpdates({ ws, project, id }: { ws: string; project: Project; id: string }) {
  const updates = useProjectUpdates(project.id);
  const mentions = (updates.data ?? []).filter((u) => mentionsMilestone(u.body, id));
  if (mentions.length === 0) return null;
  return (
    <section aria-label="Status updates" className="flex flex-col gap-2">
      <h2 className="text-sm font-medium">Status updates</h2>
      <ul className="flex flex-col gap-2">
        {mentions.map((u) => (
          <li key={u.id} className="rounded-md border border-border p-3">
            <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
              <HealthBadge health={u.health} />
              <span>{u.user?.name ?? "Someone"}</span>
              <Link
                to={`/${ws}/project/${project.identifier}/updates` as "/"}
                className="hover:underline"
              >
                {relativeTime(u.createdAt)}
              </Link>
            </div>
            <RichEditor key={u.id} value={u.body} editable={false} ariaLabel="Update" />
          </li>
        ))}
      </ul>
    </section>
  );
}

function CompleteDialog({
  project,
  milestone: m,
  onClose,
}: {
  project: Project;
  milestone: ProjectMilestone;
  onClose: () => void;
}) {
  const complete = useCompleteMilestone(project.id);
  const others = project.milestones.filter((o) => o.id !== m.id && !o.completedAt);
  const open = Math.max(0, m.issueCount - m.completedIssueCount);
  const [choice, setChoice] = useState<"keep" | "remove" | "move">(
    others.length ? "move" : "remove",
  );
  const [moveTo, setMoveTo] = useState(others[0]?.id ?? "");
  const [note, setNote] = useState("");
  const option = (value: typeof choice, label: string, hint?: string) => (
    <label className="flex cursor-default gap-2 rounded-md border border-border p-2 text-sm has-checked:border-ring">
      <input
        type="radio"
        name="open-issues"
        className="mt-0.5"
        checked={choice === value}
        onChange={() => setChoice(value)}
      />
      <span className="flex min-w-0 flex-1 flex-col">
        {label}
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </span>
    </label>
  );
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Complete {m.name}</DialogTitle>
          <DialogDescription>
            {m.completedIssueCount} of {m.issueCount} issues are done.
            {open > 0 && ` What should happen to the ${open} open issues?`}
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            complete.mutate(
              {
                id: m.id,
                input: {
                  openIssues: open > 0 ? choice : "keep",
                  moveTo: open > 0 && choice === "move" ? moveTo : undefined,
                  note: note.trim() || undefined,
                },
              },
              { onSuccess: onClose },
            );
          }}
        >
          {open > 0 && (
            <div role="radiogroup" aria-label="Open issues" className="flex flex-col gap-2">
              {others.length > 0 && option("move", "Move to another milestone")}
              {others.length > 0 && choice === "move" && (
                <select
                  aria-label="Target milestone"
                  className={selectClass}
                  value={moveTo}
                  onChange={(e) => setMoveTo(e.target.value)}
                >
                  {others.map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.name}
                    </option>
                  ))}
                </select>
              )}
              {option("remove", "Remove from milestone", "They stay in the project.")}
              {option("keep", "Keep here", "They stay listed as open on the completed milestone.")}
            </div>
          )}
          <label className="flex flex-col gap-1 text-sm">
            Result note
            <textarea
              aria-label="Result note"
              rows={3}
              value={note}
              onChange={(e) => setNote(e.target.value)}
              className="w-full resize-y rounded-md border border-input bg-transparent p-2 text-sm outline-none focus-visible:border-ring"
            />
          </label>
          <p className="text-xs text-muted-foreground">
            The issue list is saved as it is now, so the milestone stays readable later.
          </p>
          {complete.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(complete.error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={complete.isPending}>
              Complete milestone
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function TargetLine({ milestone: m }: { milestone: ProjectMilestone }) {
  const moves = m.targetDateHistory.filter((c) => c.fromDate && c.toDate).length;
  const shift = targetShift(m);
  const parts = [
    m.targetDate ? `Target ${shortDate(m.targetDate)}` : "No target date",
    moves > 0
      ? `moved ${moves === 1 ? "once" : `${moves} times`}${shift ? ` (${shiftLabel(shift)})` : ""}`
      : null,
    m.completedAt ? `completed ${relativeTime(m.completedAt)}` : null,
  ];
  return <p className="text-sm text-muted-foreground">{parts.filter(Boolean).join(" · ")}</p>;
}

function DateHistory({ milestone: m }: { milestone: ProjectMilestone }) {
  if (!m.targetDateHistory.length) return null;
  const newest = [...m.targetDateHistory].reverse();
  return (
    <section aria-label="Target date history" className="flex flex-col gap-1">
      <h2 className="text-xs font-medium text-muted-foreground">Target date history</h2>
      <ul className="flex flex-col gap-1 text-sm">
        {newest.map((c, i) => (
          <li key={c.changedAt} className="flex justify-between gap-2">
            <span className={cn(i > 0 && "text-muted-foreground line-through")}>
              {c.toDate ? shortDate(c.toDate) : "No date"}
            </span>
            <span className="text-xs text-muted-foreground">{relativeTime(c.changedAt)}</span>
          </li>
        ))}
        {newest.at(-1)?.fromDate && (
          <li className="flex justify-between gap-2 text-muted-foreground line-through">
            {shortDate(newest.at(-1)!.fromDate)}
          </li>
        )}
      </ul>
    </section>
  );
}

function Detail({
  ws,
  project,
  milestone: m,
}: {
  ws: string;
  project: Project;
  milestone: ProjectMilestone;
}) {
  const upd = useUpdateMilestone(project.id);
  const del = useDeleteMilestone(project.id);
  const reopen = useReopenMilestone(project.id);
  const convert = useConvertMilestone(project.id);
  const [completing, setCompleting] = useState(false);
  const navigate = useNavigate();
  const projectPath = `/${ws}/project/${project.identifier}`;
  const error = upd.error ?? del.error ?? reopen.error ?? convert.error;
  useRemindAction({ targetType: "milestone", target: m.id, label: m.name });
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        docTitle={m.name}
        title={
          <>
            <Link to={`${projectPath}/milestones` as "/"} className="hover:underline">
              {project.name}
            </Link>
            <span className="text-muted-foreground"> / {m.name}</span>
          </>
        }
        actions={
          <>
            {m.completedAt ? (
              <Button variant="outline" size="sm" onClick={() => reopen.mutate(m.id)}>
                Reopen
              </Button>
            ) : (
              <Button variant="outline" size="sm" onClick={() => setCompleting(true)}>
                <Check /> Complete milestone
              </Button>
            )}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon" aria-label={`Milestone actions ${m.name}`}>
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  onSelect={() =>
                    requestRemind({ targetType: "milestone", target: m.id, label: m.name })
                  }
                >
                  Remind me…
                </DropdownMenuItem>
                {!m.completedAt && (
                  <DropdownMenuItem
                    onSelect={() => {
                      if (
                        window.confirm(
                          `Turn "${m.name}" into a project? Its issues and resources move along, and the milestone is removed.`,
                        )
                      )
                        // Promise statt onSuccess: der Sync entfernt den Meilenstein oft vorher,
                        // und ein Callback von `mutate` entfällt, sobald die Seite abgebaut ist.
                        void convert.mutateAsync(m.id).then(
                          (identifier) =>
                            navigate({ to: `/${ws}/project/${identifier}/overview` as "/" }),
                          () => undefined,
                        );
                    }}
                  >
                    Convert to project…
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem
                  onSelect={() => {
                    if (
                      window.confirm(
                        `Delete milestone "${m.name}"? Its issues stay in the project without a milestone.`,
                      )
                    )
                      void del.mutateAsync(m.id).then(
                        () => navigate({ to: `${projectPath}/milestones` as "/" }),
                        () => undefined,
                      );
                  }}
                >
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      />
      {completing && (
        <CompleteDialog project={project} milestone={m} onClose={() => setCompleting(false)} />
      )}
      <div className="flex min-h-0 flex-1 flex-col overflow-auto lg:flex-row lg:overflow-hidden">
        <div className="min-w-0 flex-1 lg:overflow-auto">
          <div className="mx-auto flex max-w-3xl flex-col gap-6 p-6">
            <header className="flex items-center gap-2">
              <Diamond className={cn("size-5 shrink-0 fill-current", MILESTONE_COLOR[m.status])} />
              <h2 className="min-w-0 truncate text-xl font-semibold">{m.name}</h2>
              <MilestoneStatusBadge status={m.status} />
            </header>
            <div className="-mt-5">
              <TargetLine milestone={m} />
            </div>
            <Summary milestone={m} />
            {m.readyToComplete && (
              <p className="rounded-md border border-border p-2 text-sm text-muted-foreground">
                All issues are done. Complete the milestone when its condition is met.
              </p>
            )}
            {m.resultNote && (
              <section aria-label="Result" className="rounded-md border border-border p-3">
                <h2 className="mb-1 text-xs font-medium text-muted-foreground">Result</h2>
                <RichEditor value={m.resultNote} editable={false} ariaLabel="Result note" />
              </section>
            )}
            {error && (
              <p role="alert" className="text-xs text-destructive">
                {errorText(error)}
              </p>
            )}
            <MilestoneDescription
              key={m.id}
              id={m.id}
              value={m.description ?? ""}
              onSave={(description) => upd.mutateAsync({ id: m.id, input: { description } })}
            />
            {m.completedAt && m.hasSnapshot ? (
              <FrozenIssues ws={ws} project={project} id={m.id} />
            ) : (
              <MilestoneIssues ws={ws} project={project} milestone={m} />
            )}
            <MilestoneUpdates ws={ws} project={project} id={m.id} />
            <CommentsSection target={{ projectMilestoneId: m.id }} title="Activity" />
          </div>
        </div>
        <aside
          aria-label="Properties"
          className="flex w-full shrink-0 flex-col gap-4 border-t border-border p-4 lg:w-72 lg:overflow-auto lg:border-t-0 lg:border-l"
        >
          <TextField
            label="Milestone name"
            value={m.name}
            required
            maxLength={255}
            onCommit={(name) => upd.mutate({ id: m.id, input: { name } })}
          />
          <DateField
            label="Target date"
            date={m.targetDate}
            dayOnly
            onChange={(targetDate) => upd.mutate({ id: m.id, input: { targetDate } })}
          />
          <div className="flex flex-col gap-1">
            <span className="text-xs text-muted-foreground">Project</span>
            <Link to={`${projectPath}/overview` as "/"} className="text-sm hover:underline">
              {project.name}
            </Link>
          </div>
          <ResourcesSection ws={ws} projectId={project.id} milestoneId={m.id} />
          <DateHistory milestone={m} />
        </aside>
      </div>
    </div>
  );
}

export function MilestonePage({
  ws,
  projectRef,
  milestoneId,
}: {
  ws: string;
  projectRef: string;
  milestoneId: string;
}) {
  const q = useProject(projectRef);
  if (q.isLoading) return <ListSkeleton rows={6} />;
  if (q.isError || !q.data) {
    return (
      <ErrorState
        title="Could not load the milestone"
        error={q.error}
        onRetry={() => void q.refetch()}
      />
    );
  }
  const milestone = q.data.milestones.find((m) => m.id === milestoneId);
  if (!milestone) {
    return (
      <div className="flex flex-col items-center gap-2 p-10 text-sm">
        <p className="font-medium">Milestone not found</p>
        <Link
          to={`/${ws}/project/${projectRef}/milestones` as "/"}
          className="text-muted-foreground hover:underline"
        >
          Back to milestones
        </Link>
      </div>
    );
  }
  return <Detail ws={ws} project={q.data} milestone={milestone} />;
}
