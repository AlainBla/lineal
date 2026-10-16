import { Link } from "@tanstack/react-router";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/cn";
import type { Issue } from "@/gql/graphql";
import { DEFAULT_PROPERTIES, type DisplayProperty } from "@/features/views/display-options";

import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { estimateLabel, issuePath, priorityLabel, relativeTime } from "./model";
import type { Selection } from "./selection";

interface RowProps {
  issue: Issue;
  ws: string;
  lookup: IssueLookup;
  focused: boolean;
  onFocus: (id: string) => void;
  /** Mehrfachauswahl (Shift-/Cmd-Klick, Checkbox); fehlt sie, ist die Zeile nicht auswählbar. */
  selection?: Selection;
  /** Sichtbare Eigenschaften (Display properties); Standard wie bisher. */
  display?: readonly DisplayProperty[];
  /** Einrückung für verschachtelte Sub-Issues. */
  depth?: number;
}

const shortDay = (iso: string) =>
  new Date(iso.length === 10 ? `${iso}T00:00:00Z` : iso).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  });

/** Checkbox am Zeilenanfang: erscheint beim Überfahren, bei Fokus oder sobald etwas gewählt ist. */
export function SelectBox({ issue, selection }: { issue: Issue; selection: Selection }) {
  const selected = selection.has(issue.id);
  return (
    <input
      type="checkbox"
      aria-label={`Select ${issue.identifier}`}
      checked={selected}
      readOnly
      className={cn(
        "size-3.5 shrink-0 cursor-pointer accent-primary focus-visible:opacity-100",
        !selected && selection.size === 0 && "opacity-0 group-hover:opacity-100",
      )}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        selection.toggle(issue.id);
      }}
    />
  );
}

function LabelChips({
  issue,
  lookup,
  max = 3,
  nowrap = false,
}: {
  issue: Issue;
  lookup: IssueLookup;
  max?: number;
  /** Einzeilig: lange Namen werden abgeschnitten (Board-Karte). */
  nowrap?: boolean;
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
          className={cn(
            "inline-flex items-center gap-1 rounded-full border border-border px-1.5 text-xs text-muted-foreground",
            nowrap && "max-w-24 shrink-0",
          )}
        >
          <LabelDot color={l.color} />
          {nowrap ? <span className="truncate">{l.name}</span> : l.name}
        </span>
      ))}
      {labels.length > max && (
        <span className="shrink-0 text-xs text-subtle-foreground">+{labels.length - max}</span>
      )}
    </>
  );
}

/** Zeile der Listenansicht: Priority, ID, Status, Titel, Labels, Projekt, Assignee, Datum (docs/08 §2.1). */
export function IssueRow({
  issue,
  ws,
  lookup,
  focused,
  onFocus,
  selection,
  display = DEFAULT_PROPERTIES,
  depth = 0,
}: RowProps) {
  const show = (p: DisplayProperty) => display.includes(p);
  const creator = show("creator") ? lookup.assigneeName(issue.creatorId ?? null) : null;
  const cycle = show("cycle") ? lookup.cycleName(issue.cycleId ?? null) : null;
  const selected = selection?.has(issue.id) ?? false;
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  const project = lookup.projectName(issue.projectId);
  const milestone = lookup.milestoneName(issue.projectMilestoneId);
  const estimate = estimateLabel(lookup.estimationType(issue.teamId), issue.estimate);
  return (
    <Link
      to={issuePath(ws, issue.identifier) as "/"}
      data-issue-id={issue.id}
      data-focused={focused || undefined}
      data-selected={selected || undefined}
      onFocus={() => onFocus(issue.id)}
      onClick={(e) => {
        if (selection?.handleClick(issue.id, e)) e.preventDefault();
      }}
      className={cn(
        "group flex h-9 items-center gap-3 border-b border-border px-4 text-sm outline-none hover:bg-surface-hover focus-visible:bg-surface-hover focus-visible:outline-none",
        focused && "bg-surface-hover",
        selected && "bg-surface-active",
      )}
    >
      {selection && <SelectBox issue={issue} selection={selection} />}
      {depth > 0 && <span aria-hidden style={{ width: depth * 20 }} className="shrink-0" />}
      {show("priority") && <PriorityIcon priority={issue.priority} />}
      {show("id") && (
        <span className="w-16 shrink-0 text-xs text-muted-foreground tabular-nums">
          {issue.identifier}
        </span>
      )}
      {show("status") && (
        <StatusIcon type={issue.stateType} color={state?.color} title={issue.stateName} />
      )}
      <span className="min-w-0 flex-1 truncate">{issue.title}</span>
      {show("labels") && (
        <span className="hidden items-center gap-1 md:flex">
          <LabelChips issue={issue} lookup={lookup} />
        </span>
      )}
      {cycle && <span className="hidden text-xs text-muted-foreground lg:inline">{cycle}</span>}
      {show("links") && issue.linkCount > 0 && (
        <span className="text-xs text-muted-foreground" title="Links">
          {issue.linkCount} links
        </span>
      )}
      {show("timeInStatus") && (
        <span className="text-xs text-muted-foreground" title="Time in status">
          {relativeTime(issue.stateEnteredAt)}
        </span>
      )}
      {show("dueDate") && issue.dueDate && (
        <span className="text-xs text-muted-foreground" title="Due date">
          {shortDay(issue.dueDate)}
        </span>
      )}
      {(show("milestone") ? project : null) && (
        <span className="hidden max-w-32 truncate text-xs text-muted-foreground lg:inline">
          {project}
          {milestone && <span title="Milestone"> · {milestone}</span>}
        </span>
      )}
      {estimate && <span className="text-xs text-muted-foreground">{estimate}</span>}
      {creator && (
        <span title={`Creator: ${creator}`}>
          <Avatar name={creator} />
        </span>
      )}
      {!show("assignee") ? null : assignee ? (
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
      {show("created") && (
        <span className="w-8 shrink-0 text-right text-xs text-subtle-foreground" title="Created">
          {relativeTime(issue.createdAt)}
        </span>
      )}
      {show("updated") && (
        <span className="w-8 shrink-0 text-right text-xs text-subtle-foreground" title="Updated">
          {relativeTime(issue.updatedAt)}
        </span>
      )}
    </Link>
  );
}

/**
 * Inhalt einer Board-Karte. Feste Höhe, damit alle Karten gleich hoch sind: Titel einzeilig mit
 * Ellipsis, Label-Zeile einzeilig (Überlauf als „+N“); Vollständiges zeigt `IssueCardHover`.
 */
export function IssueCardBody({ issue, lookup }: { issue: Issue; lookup: IssueLookup }) {
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  return (
    <div className="flex h-[4.5rem] flex-col gap-2 overflow-hidden" data-testid="issue-card-body">
      <div className="flex h-4 items-center gap-2 text-xs text-muted-foreground">
        <PriorityIcon priority={issue.priority} />
        <span className="tabular-nums">{issue.identifier}</span>
        <span className="ml-auto">
          {assignee ? (
            <span>
              <Avatar name={assignee} />
            </span>
          ) : null}
        </span>
      </div>
      <div className="flex h-5 items-center gap-2">
        <StatusIcon type={issue.stateType} color={state?.color} className="shrink-0" />
        <span className="min-w-0 flex-1 truncate text-sm leading-snug" data-testid="card-title">
          {issue.title}
        </span>
      </div>
      <div className="flex h-5 flex-nowrap items-center gap-1 overflow-hidden">
        <LabelChips issue={issue} lookup={lookup} max={2} nowrap />
      </div>
    </div>
  );
}

/** Hover-Details (wie Linear): alles, was die Karte abkürzt, plus Status, Priorität und Assignee. */
export function IssueCardHover({
  issue,
  lookup,
  children,
}: {
  issue: Issue;
  lookup: IssueLookup;
  children: React.ReactElement;
}) {
  const assignee = lookup.assigneeName(issue.assigneeId);
  const labels = issue.labelIds.flatMap((id) => {
    const l = lookup.label(id);
    return l ? [l] : [];
  });
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent
        side="right"
        align="start"
        data-testid="card-hover"
        className="max-w-80 flex-col items-start gap-1.5 p-2.5"
      >
        <span className="text-muted-foreground tabular-nums">{issue.identifier}</span>
        <span className="text-sm font-medium break-words whitespace-normal">{issue.title}</span>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-muted-foreground">
          <dt>Status</dt>
          <dd className="text-popover-foreground">{issue.stateName}</dd>
          <dt>Priority</dt>
          <dd className="text-popover-foreground">{priorityLabel(issue.priority)}</dd>
          <dt>Assignee</dt>
          <dd className="text-popover-foreground">{assignee ?? "Unassigned"}</dd>
        </dl>
        {labels.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {labels.map((l) => (
              <span
                key={l.id}
                className="inline-flex items-center gap-1 rounded-full border border-border px-1.5"
              >
                <LabelDot color={l.color} />
                {l.name}
              </span>
            ))}
          </div>
        )}
      </TooltipContent>
    </Tooltip>
  );
}
