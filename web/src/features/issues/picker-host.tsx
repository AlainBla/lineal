import { useEffect, useState } from "react";

import { useSession } from "@/app/auth";
import type { Issue } from "@/gql/graphql";

import { useTeamMeta } from "./hooks";
import { useIssueActions } from "./issue-actions";
import { toggleLabel } from "./model";
import { PropertyPicker } from "./picker";
import {
  assigneeOptions,
  cycleOptions,
  estimatePickerOptions,
  labelOptions,
  NONE,
  priorityOptions,
  projectOptions,
  statusOptions,
} from "./property-options";
import { DueDateDialog } from "./due-date-dialog";
import { TeamSwitchDialog } from "./team-switch-dialog";
import { errorMessage, showToast } from "./toast-store";
import { useKnownUsers } from "./users";

export type PickerKind =
  "status" | "assignee" | "priority" | "labels" | "estimate" | "project" | "cycle" | "team" | "due";

export interface PickerRequest {
  kind: PickerKind;
  issue: Issue;
}

const TITLES: Record<Exclude<PickerKind, "team" | "due">, string> = {
  status: "Change status…",
  assignee: "Assign to…",
  priority: "Set priority…",
  labels: "Change labels…",
  estimate: "Set estimate…",
  project: "Add to project…",
  cycle: "Add to cycle…",
};

/**
 * Öffnet je nach Anfrage den passenden Picker für ein Issue und wendet die Auswahl optimistisch an.
 * Eine Instanz je Screen; Liste, Board und Detail setzen nur `request`.
 */
export function IssuePickerHost({
  request,
  onClose,
}: {
  request: PickerRequest | null;
  onClose: () => void;
}) {
  const issue = request?.issue;
  const kind = request?.kind;
  const meta = useTeamMeta(issue?.teamId);
  const actions = useIssueActions();
  const session = useSession();
  const users = useKnownUsers();
  // Mehrfachauswahl: lokale Zwischenstände, bis der Cache die Änderung zeigt.
  const [labelOverride, setLabelOverride] = useState<{ id: string; ids: string[] } | null>(null);
  const labelIds =
    issue && labelOverride?.id === issue.id ? labelOverride.ids : (issue?.labelIds ?? []);

  const estimationType = meta.data?.team.issueEstimationType ?? "notUsed";
  const cyclesEnabled = meta.data?.team.cyclesEnabled ?? false;

  // Nicht verfügbare Felder: Hinweis statt leerem Picker (docs/08 F4).
  const unavailable =
    kind === "estimate" && meta.data && estimationType === "notUsed"
      ? "Estimates are not enabled for this team."
      : kind === "cycle" && meta.data && !cyclesEnabled
        ? "Cycles are not enabled for this team."
        : null;
  // Teamdaten nicht ladbar: Fehler statt „No results.“ (B4.27).
  const metaFailed =
    meta.isError &&
    !!kind &&
    kind !== "team" &&
    kind !== "priority" &&
    kind !== "assignee" &&
    kind !== "due";
  useEffect(() => {
    if (metaFailed) {
      showToast({ message: errorMessage(meta.error, "Could not load the team."), tone: "error" });
      onClose();
    }
  }, [metaFailed, meta.error, onClose]);
  useEffect(() => {
    if (unavailable) {
      showToast({ message: unavailable });
      onClose();
    }
  }, [unavailable, onClose]);

  if (!request || !issue || !kind) return null;
  if (kind === "due") {
    return (
      <DueDateDialog
        key={issue.id}
        open
        current={issue.dueDate}
        onClose={onClose}
        onSave={(d) => actions.setDueDate(issue, d)}
      />
    );
  }
  if (kind === "team") return <TeamSwitchDialog issue={issue} onClose={onClose} />;

  const me = session ? { id: session.user.id, name: session.user.name } : null;
  const states = meta.data?.states ?? [];
  const common = {
    open: !unavailable && !metaFailed,
    onOpenChange: (o: boolean) => !o && onClose(),
    title: TITLES[kind],
  };

  switch (kind) {
    case "status":
      return (
        <PropertyPicker
          {...common}
          options={statusOptions(states)}
          selected={issue.stateId}
          onSelect={(id) => actions.setState(issue, id, states)}
        />
      );
    case "priority":
      return (
        <PropertyPicker
          {...common}
          options={priorityOptions()}
          selected={String(issue.priority)}
          onSelect={(id) => actions.setPriority(issue, Number(id))}
        />
      );
    case "assignee":
      return (
        <PropertyPicker
          {...common}
          options={assigneeOptions(users, me)}
          selected={issue.assigneeId ?? NONE}
          onSelect={(id) => actions.setAssignee(issue, id === NONE ? null : id)}
        />
      );
    case "labels": {
      const all = meta.data?.labels ?? [];
      return (
        <PropertyPicker
          {...common}
          multi
          options={labelOptions(all)}
          selected={labelIds}
          onSelect={(id) => {
            const next = toggleLabel(labelIds, id, all);
            setLabelOverride({ id: issue.id, ids: next });
            actions.setLabels(issue, next);
          }}
        />
      );
    }
    case "estimate":
      return (
        <PropertyPicker
          {...common}
          options={estimatePickerOptions(estimationType)}
          selected={issue.estimate === null ? NONE : String(issue.estimate)}
          onSelect={(id) => actions.setEstimate(issue, id === NONE ? null : Number(id))}
        />
      );
    case "project":
      return (
        <PropertyPicker
          {...common}
          options={projectOptions(meta.data?.projects ?? [])}
          selected={issue.projectId ?? NONE}
          onSelect={(id) => actions.setProject(issue, id === NONE ? null : id)}
        />
      );
    case "cycle":
      return (
        <PropertyPicker
          {...common}
          options={cycleOptions(meta.data?.cycles ?? [])}
          selected={issue.cycleId ?? NONE}
          onSelect={(id) => actions.setCycle(issue, id === NONE ? null : id)}
        />
      );
  }
}
