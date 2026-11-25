import type { ReactNode } from "react";

import type { Issue } from "@/gql/graphql";

import { Avatar, LabelChip, PriorityIcon, StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { priorityLabel } from "./model";
import type { PickerKind } from "./picker-host";

/** Chip der kompakten Zeile: Button, wenn sich ein Picker öffnen lässt; sonst nur Anzeige. */
function Chip({
  onOpen,
  label,
  children,
}: {
  onOpen?: () => void;
  /** Barrierefreier Name des Buttons (z. B. „Change status“). */
  label: string;
  children: ReactNode;
}) {
  const className =
    "flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs whitespace-nowrap";
  return onOpen ? (
    <button
      type="button"
      onClick={onOpen}
      aria-label={label}
      className={`${className} hover:bg-surface-hover focus-visible:outline-2`}
    >
      {children}
    </button>
  ) : (
    <span className={className}>{children}</span>
  );
}

/**
 * LIN-92: Kompakte, horizontal scrollbare Eigenschaften-Zeile für schmale Breiten (< `md`). Zeigt
 * Status, Priorität, Zuständigen und Labels sofort über dem Titel – ohne das Panel zu öffnen. Tipp
 * auf einen Chip öffnet denselben Picker wie die `PropertySidebar` (`onOpen(kind)` → `setRequest`).
 * Ab `md` wird die Zeile nicht gerendert (der Aufrufer prüft `narrow`), damit nichts doppelt steht.
 */
export function MobilePropertyRow({
  issue,
  lookup,
  onOpen,
  readOnly = false,
  metaError = false,
}: {
  issue: Issue;
  lookup: IssueLookup;
  onOpen: (kind: PickerKind) => void;
  /** Geteiltes Issue ohne Teamzugriff: Chips nur anzeigen (B4.16). */
  readOnly?: boolean;
  /** Teamdaten nicht geladen: keine Picker öffnen (B4.27). */
  metaError?: boolean;
}) {
  const open = (kind: PickerKind) => (readOnly || metaError ? undefined : () => onOpen(kind));
  const state = lookup.state(issue.stateId);
  const assignee = lookup.assigneeName(issue.assigneeId);
  const labels = issue.labelIds.flatMap((id) => {
    const l = lookup.label(id);
    return l ? [l] : [];
  });

  return (
    <div
      aria-label="Issue properties"
      className="flex shrink-0 [scrollbar-width:none] items-center gap-1.5 overflow-x-auto border-b border-border px-4 py-1.5 [&::-webkit-scrollbar]:hidden"
    >
      <Chip onOpen={open("status")} label="Change status">
        <StatusIcon type={issue.stateType} color={state?.color} title={issue.stateName} />
        <span>{issue.stateName}</span>
      </Chip>
      <Chip onOpen={open("priority")} label="Change priority">
        <PriorityIcon priority={issue.priority} />
        <span>{priorityLabel(issue.priority)}</span>
      </Chip>
      <Chip onOpen={open("assignee")} label={assignee ? `Assignee: ${assignee}` : "Assign"}>
        {issue.assigneeId ? (
          <>
            <Avatar name={assignee ?? "?"} />
            <span>{assignee ?? "Assigned"}</span>
          </>
        ) : (
          <span className="text-muted-foreground">Assign</span>
        )}
      </Chip>
      <Chip onOpen={open("labels")} label="Change labels">
        {labels.length > 0 ? (
          <span className="flex items-center gap-1">
            {labels.map((l) => (
              <LabelChip key={l.id} name={l.name} color={l.color} />
            ))}
          </span>
        ) : (
          <span className="text-muted-foreground">Add label</span>
        )}
      </Chip>
    </div>
  );
}
