import { Link } from "@tanstack/react-router";

import { cn } from "@/lib/cn";
import type { Issue } from "@/gql/graphql";

import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { estimateLabel, issuePath, relativeTime } from "./model";

interface RowProps {
  issue: Issue;
  ws: string;
  lookup: IssueLookup;
  focused: boolean;
  onFocus: (id: string) => void;
}

function LabelChips({
  issue,
  lookup,
  max = 3,
}: {
  issue: Issue;
  lookup: IssueLookup;
  max?: number;
}) {
  const labels = issue.labelIds.flatMap((id) => {
    const l = lookup.label(id);
    return l ? [l] : [];
  });
  return (
    <>
      {labels.slice(0, max).map((l) => (
        <span
          key={l.id}
          className="inline-flex items-center gap-1 rounded-full border border-border px-1.5 text-xs text-muted-foreground"
        >
          <LabelDot color={l.color} />
          {l.name}
        </span>
      ))}
      {labels.length > max && (
        <span className="text-xs text-subtle-foreground">+{labels.length - max}</span>
      )}
    </>
  );
}

/** Zeile der Listenansicht: Priority, ID, Status, Titel, Labels, Projekt, Assignee, Datum (docs/08 §2.1). */
export function IssueRow({ issue, ws, lookup, focused, onFocus }: RowProps) {
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  const project = lookup.projectName(issue.projectId);
  const estimate = estimateLabel(lookup.estimationType(issue.teamId), issue.estimate);
  return (
    <Link
      to={issuePath(ws, issue.identifier) as "/"}
      data-issue-id={issue.id}
      data-focused={focused || undefined}
      onFocus={() => onFocus(issue.id)}
      className={cn(
        "group flex h-9 items-center gap-3 border-b border-border px-4 text-sm outline-none hover:bg-surface-hover focus-visible:bg-surface-hover focus-visible:outline-none",
        focused && "bg-surface-hover",
      )}
    >
      <PriorityIcon priority={issue.priority} />
      <span className="w-16 shrink-0 text-xs text-muted-foreground tabular-nums">
        {issue.identifier}
      </span>
      <StatusIcon type={issue.stateType} color={state?.color} title={issue.stateName} />
      <span className="min-w-0 flex-1 truncate">{issue.title}</span>
      <span className="hidden items-center gap-1 md:flex">
        <LabelChips issue={issue} lookup={lookup} />
      </span>
      {project && (
        <span className="hidden max-w-32 truncate text-xs text-muted-foreground lg:inline">
          {project}
        </span>
      )}
      {estimate && <span className="text-xs text-muted-foreground">{estimate}</span>}
      {assignee ? (
        <span title={assignee}>
          <Avatar name={assignee} />
        </span>
      ) : issue.assigneeId ? (
        <span title="Assigned">
          <Avatar name="?" />
        </span>
      ) : (
        <span
          className="size-[18px] rounded-full border border-dashed border-border-strong"
          title="Unassigned"
        />
      )}
      <span className="w-8 shrink-0 text-right text-xs text-subtle-foreground">
        {relativeTime(issue.updatedAt)}
      </span>
    </Link>
  );
}

/** Karte des Boards. */
export function IssueCardBody({ issue, lookup }: { issue: Issue; lookup: IssueLookup }) {
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <PriorityIcon priority={issue.priority} />
        <span className="tabular-nums">{issue.identifier}</span>
        <span className="ml-auto">
          {assignee ? (
            <span title={assignee}>
              <Avatar name={assignee} />
            </span>
          ) : null}
        </span>
      </div>
      <div className="flex items-start gap-2">
        <StatusIcon
          type={issue.stateType}
          color={state?.color}
          title={issue.stateName}
          className="mt-0.5"
        />
        <span className="text-sm leading-snug">{issue.title}</span>
      </div>
      {issue.labelIds.length > 0 && (
        <div className="flex flex-wrap items-center gap-1">
          <LabelChips issue={issue} lookup={lookup} max={2} />
        </div>
      )}
    </div>
  );
}
