import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";

import { Keys } from "@/components/ui/kbd";
import type { Issue } from "@/gql/graphql";
import { teamPaths } from "@/app/nav";

import { Avatar, LabelDot, PriorityIcon, StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { estimateLabel, issuePath, priorityLabel } from "./model";
import type { PickerKind } from "./picker-host";
import type { TeamMeta } from "./queries";

interface PropertyRowProps {
  label: string;
  keys?: string;
  /** Test-/A11y-Name des Buttons. */
  onOpen?: () => void;
  children: ReactNode;
}

function PropertyRow({ label, keys, onOpen, children }: PropertyRowProps) {
  return (
    <div className="flex min-h-8 items-center gap-2" data-property={label}>
      <dt className="w-20 shrink-0 text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 flex-1">
        {onOpen ? (
          <button
            type="button"
            onClick={onOpen}
            aria-label={`${label}: change`}
            className="group flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-surface-hover focus-visible:outline-2"
          >
            {children}
            {keys && <Keys keys={keys} className="ml-auto opacity-0 group-hover:opacity-100" />}
          </button>
        ) : (
          <div className="flex items-center gap-2 px-2 py-1 text-sm">{children}</div>
        )}
      </dd>
    </div>
  );
}

interface SidebarProps {
  issue: Issue;
  meta: TeamMeta | undefined;
  lookup: IssueLookup;
  ws: string;
  onOpen: (kind: PickerKind) => void;
  onEditDueDate: () => void;
  /** Geteiltes Issue ohne Teamzugriff: Eigenschaften nur anzeigen (B4.16). */
  readOnly?: boolean;
  /** Teamdaten konnten nicht geladen werden (z. B. 429): Hinweis mit Retry statt „Team“ (B4.27). */
  metaError?: boolean;
  onRetryMeta?: () => void;
  /** Aufgelöstes Parent-Issue (Kennung und Titel), falls vorhanden und sichtbar. */
  parent?: Pick<Issue, "identifier" | "title"> | null;
}

/**
 * Property-Sidebar in der Reihenfolge aus docs/08 §2.2: Status, Team, Priority, Assignee, Cycle,
 * Estimate, Due date, Labels, Project (Milestone und Related folgen im Hauptbereich bzw. entfallen mangels Daten).
 */
export function PropertySidebar({
  issue,
  meta,
  lookup,
  ws,
  onOpen,
  onEditDueDate,
  readOnly = false,
  metaError = false,
  onRetryMeta,
  parent,
}: SidebarProps) {
  const open = (f: () => void) => (readOnly || metaError ? undefined : f);
  const teamKeyFromIdentifier = issue.identifier.split("-")[0] ?? "";
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  const cycle = meta?.cycles.find((c) => c.id === issue.cycleId);
  const estimationType = meta?.team.issueEstimationType ?? "notUsed";
  const labels = issue.labelIds.flatMap((id) => {
    const l = lookup.label(id);
    return l ? [l] : [];
  });
  const project = lookup.projectName(issue.projectId);

  return (
    <dl aria-label="Properties" className="flex flex-col gap-0.5">
      <PropertyRow label="Status" keys="s" onOpen={open(() => onOpen("status"))}>
        <StatusIcon type={issue.stateType} color={state?.color} title={issue.stateName} />
        <span>{issue.stateName}</span>
      </PropertyRow>
      <PropertyRow label="Team" keys="mod+shift+m" onOpen={open(() => onOpen("team"))}>
        {metaError ? (
          <span className="flex items-center gap-2 text-destructive" role="alert">
            Could not load team
            {onRetryMeta && (
              <button
                type="button"
                className="text-xs underline"
                onClick={(e) => {
                  e.stopPropagation();
                  onRetryMeta();
                }}
              >
                Retry
              </button>
            )}
          </span>
        ) : (
          <span>{meta?.team.name ?? teamKeyFromIdentifier}</span>
        )}
        {meta && (
          <Link
            to={teamPaths(ws, meta.team.key).home as "/"}
            className="text-xs text-muted-foreground hover:underline"
            onClick={(e) => e.stopPropagation()}
          >
            {meta.team.key}
          </Link>
        )}
      </PropertyRow>
      <PropertyRow label="Priority" keys="p" onOpen={open(() => onOpen("priority"))}>
        <PriorityIcon priority={issue.priority} />
        <span>{priorityLabel(issue.priority)}</span>
      </PropertyRow>
      <PropertyRow label="Assignee" keys="a" onOpen={open(() => onOpen("assignee"))}>
        {issue.assigneeId ? (
          <>
            <Avatar name={assignee ?? "?"} />
            <span>{assignee ?? "Assigned"}</span>
          </>
        ) : (
          <span className="text-muted-foreground">Assign</span>
        )}
      </PropertyRow>
      {meta?.team.cyclesEnabled && (
        <PropertyRow label="Cycle" keys="shift+c" onOpen={open(() => onOpen("cycle"))}>
          {cycle ? (
            <span>Cycle {cycle.number}</span>
          ) : (
            <span className="text-muted-foreground">Add to cycle</span>
          )}
        </PropertyRow>
      )}
      {estimationType !== "notUsed" && (
        <PropertyRow label="Estimate" keys="shift+e" onOpen={open(() => onOpen("estimate"))}>
          {issue.estimate !== null ? (
            <span>{estimateLabel(estimationType, issue.estimate)}</span>
          ) : (
            <span className="text-muted-foreground">Set estimate</span>
          )}
        </PropertyRow>
      )}
      {issue.dueDate && (
        <PropertyRow label="Due date" keys="shift+d" onOpen={open(onEditDueDate)}>
          <span>{issue.dueDate}</span>
        </PropertyRow>
      )}
      {issue.slaBreachesAt && (
        <PropertyRow label="SLA">
          <span title={`Started ${issue.slaStartedAt ?? "—"}`}>
            {issue.slaType === "business" ? "Business days · " : ""}
            Breaches {new Date(issue.slaBreachesAt)
              .toISOString()
              .slice(0, 16)
              .replace("T", " ")}{" "}
            UTC
          </span>
        </PropertyRow>
      )}
      <PropertyRow label="Labels" keys="l" onOpen={open(() => onOpen("labels"))}>
        {labels.length > 0 ? (
          <span className="flex flex-wrap gap-1">
            {labels.map((l) => (
              <span
                key={l.id}
                className="inline-flex items-center gap-1 rounded-full border border-border px-1.5 text-xs"
              >
                <LabelDot color={l.color} />
                {l.name}
              </span>
            ))}
          </span>
        ) : (
          <span className="text-muted-foreground">Add label</span>
        )}
      </PropertyRow>
      <PropertyRow label="Project" keys="shift+p" onOpen={open(() => onOpen("project"))}>
        {project ? (
          <span>{project}</span>
        ) : (
          <span className="text-muted-foreground">Add to project</span>
        )}
      </PropertyRow>
      <PropertyRow label="Parent" keys="mod+shift+p" onOpen={open(() => onOpen("parent"))}>
        {parent ? (
          <Link
            to={issuePath(ws, parent.identifier) as "/"}
            className="truncate"
            onClick={(e) => e.stopPropagation()}
          >
            {parent.identifier} {parent.title}
          </Link>
        ) : issue.parentId ? (
          <span className="text-muted-foreground">Parent issue</span>
        ) : (
          <span className="text-muted-foreground">Set parent</span>
        )}
      </PropertyRow>
    </dl>
  );
}
