import { ChevronDown, ChevronRight, Diamond, MoreHorizontal, Plus } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import type { Project } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useViewIssues } from "@/features/views/queries";

import { ProjectProperties } from "./project-details";
import { useCreateMilestone, useDeleteMilestone, useUpdateMilestone } from "./queries";
import { shortDate } from "./ui";

const MILESTONE_COLOR: Record<string, string> = {
  done: "text-primary",
  overdue: "text-destructive",
  next: "text-amber-500",
  unstarted: "text-muted-foreground",
};

/** Einklappbarer Abschnitt des Seitenpanels (▾), optional mit „+“ rechts. */
function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(true);
  return (
    <section aria-label={title} className="rounded-lg border border-border bg-surface p-3">
      <div className="flex items-center justify-between">
        <button
          type="button"
          aria-expanded={open}
          aria-label={`${open ? "Collapse" : "Expand"} ${title}`}
          className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
          onClick={() => setOpen((o) => !o)}
        >
          {title}
          {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
        </button>
        {action}
      </div>
      {open && <div className="mt-3">{children}</div>}
    </section>
  );
}

function MilestonesSection({
  project,
  onSelect,
}: {
  project: Project;
  onSelect?: (id: string | null) => void;
}) {
  const create = useCreateMilestone(project.id);
  const del = useDeleteMilestone(project.id);
  const upd = useUpdateMilestone(project.id);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const inMilestones = project.milestones.reduce((n, m) => n + m.issueCount, 0);
  const without = Math.max(0, project.issueCount - inMilestones);
  const error = create.error ?? del.error ?? upd.error;
  return (
    <Section
      title="Milestones"
      action={
        <Button
          variant="ghost"
          size="icon"
          aria-label="Add milestone"
          onClick={() => setAdding((a) => !a)}
        >
          <Plus />
        </Button>
      }
    >
      <ul className="flex flex-col gap-1.5 text-sm">
        {project.milestones.map((m) => (
          <li key={m.id} className="flex items-center gap-2">
            <Diamond className={cn("size-3.5 shrink-0", MILESTONE_COLOR[m.status])} />
            <button
              type="button"
              className="min-w-0 truncate text-left hover:underline"
              onClick={() => onSelect?.(m.id)}
            >
              {m.name}
            </button>
            <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
              {Math.round(m.progress * 100)}% of {m.issueCount}
            </span>
            {m.targetDate && (
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {shortDate(m.targetDate).replace(/ \d{4}$/, "")}
              </span>
            )}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Milestone actions ${m.name}`}
                  className={m.targetDate ? undefined : "ml-auto"}
                >
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  onSelect={() => {
                    const next = window.prompt("Milestone name", m.name)?.trim();
                    if (next && next !== m.name) upd.mutate({ id: m.id, input: { name: next } });
                  }}
                >
                  Rename…
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => {
                    if (
                      window.confirm(
                        `Delete milestone "${m.name}"? Its issues stay in the project without a milestone.`,
                      )
                    )
                      del.mutate(m.id);
                  }}
                >
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </li>
        ))}
        <li className="flex items-center gap-2 text-muted-foreground">
          <Diamond className="size-3.5 shrink-0 opacity-50" />
          <button type="button" className="hover:underline" onClick={() => onSelect?.("none")}>
            No milestone
          </button>
          <span className="text-xs tabular-nums">{without}</span>
        </li>
      </ul>
      {adding && (
        <form
          aria-label="Add milestone"
          className="mt-2 flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (!name.trim()) return;
            create.mutate(
              { name: name.trim() },
              {
                onSuccess: () => {
                  setName("");
                  setAdding(false);
                },
              },
            );
          }}
        >
          <Input
            autoFocus
            aria-label="Milestone name"
            placeholder="Milestone name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <Button type="submit" size="sm" disabled={!name.trim() || create.isPending}>
            Add
          </Button>
        </form>
      )}
      {error && (
        <p role="alert" className="mt-2 text-xs text-destructive">
          {errorText(error)}
        </p>
      )}
    </Section>
  );
}

function ProgressSection({ project }: { project: Project }) {
  const started = useViewIssues({
    filter: {
      project: { id: { in: [project.id] } },
      state: { type: { in: ["started"] } },
    },
  });
  const stat = (label: string, value: ReactNode, color: string) => (
    <div className="flex flex-col gap-0.5">
      <span className="flex items-center gap-1.5 text-sm text-muted-foreground">
        <span className={cn("size-2 rounded-[2px]", color)} />
        {label}
      </span>
      <span className="text-sm tabular-nums">{value}</span>
    </div>
  );
  return (
    <Section title="Progress">
      <div className="grid grid-cols-3 gap-2">
        {stat("Scope", project.issueCount, "bg-muted-foreground")}
        {stat("Started", started.data ? started.data.issues.length : "–", "bg-amber-500")}
        {stat("Completed", project.completedIssueCount, "bg-primary")}
      </div>
    </Section>
  );
}

/**
 * Seitenpanel der Projekt-Issues (docs/08 §2.7): Properties (direkt editierbar), Milestones mit
 * Fortschritt und Progress (Scope / Started / Completed).
 */
export function ProjectPanel({
  project,
  ws,
  onSelectMilestone,
}: {
  project: Project;
  ws: string;
  onSelectMilestone?: (id: string | null) => void;
}) {
  return (
    <aside
      aria-label="Project details"
      className="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto border-l border-border p-3"
    >
      <Section title="Properties">
        <ProjectProperties project={project} ws={ws} stacked />
      </Section>
      <MilestonesSection project={project} onSelect={onSelectMilestone} />
      <ProgressSection project={project} />
    </aside>
  );
}
