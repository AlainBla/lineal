/**
 * Bulk-Aktionsleiste und -Picker für die Mehrfachauswahl (docs/08 §2.1, docs/02 §7.4): Status,
 * Priorität, Assignee, Labels hinzufügen/entfernen, Projekt, Cycle, Estimate, Team, Archivieren und
 * Löschen. Alle Aktionen gehen als ein `issueBatchUpdate` an den Server; Teilerfolge werden als
 * Toast mit den fehlgeschlagenen Kennungen gemeldet.
 */
import { Archive, Trash2, X } from "lucide-react";
import { useMemo, useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { useBatchUpdate, useTeamMeta, useTeams } from "./hooks";
import { PropertyPicker } from "./picker";
import type { PickerKind } from "./picker-host";
import {
  assigneeOptions,
  cycleOptions,
  estimatePickerOptions,
  labelOptions,
  NONE,
  priorityOptions,
  projectOptions,
  statusOptions,
  teamOptions,
} from "./property-options";
import { summarizeBatch } from "./bulk-summary";
import type { IssueBatchVars } from "./queries";
import { errorMessage, showToast } from "./toast-store";
import { useKnownUsers } from "./users";

export type BulkKind = Exclude<PickerKind, "due"> | "labels-remove" | "delete";

const TITLES: Record<Exclude<BulkKind, "delete" | "team">, string> = {
  status: "Change status…",
  assignee: "Assign to…",
  priority: "Set priority…",
  labels: "Add label…",
  "labels-remove": "Remove label…",
  estimate: "Set estimate…",
  project: "Add to project…",
  cycle: "Add to cycle…",
};

interface Props {
  /** Aktuell ausgewählte Issues. */
  issues: Issue[];
  onClear: () => void;
  /** Offener Picker bzw. Dialog (gesteuert vom Screen, damit Tastenkürzel ihn öffnen können). */
  request: BulkKind | null;
  onRequest: (kind: BulkKind | null) => void;
}

export function BulkActions({ issues, onClear, request, onRequest }: Props) {
  const batch = useBatchUpdate();
  const session = useSession();
  const users = useKnownUsers();
  const teams = useTeams();
  const teamIds = useMemo(() => [...new Set(issues.map((i) => i.teamId))], [issues]);
  const single = teamIds.length === 1;
  const meta = useTeamMeta(single ? teamIds[0] : undefined);
  const [pendingTeam, setPendingTeam] = useState<{ id: string; name: string } | null>(null);
  const [withSubs, setWithSubs] = useState(false);
  useShortcutContext("modal", pendingTeam !== null || request === "delete");

  const ids = useMemo(() => issues.map((i) => i.id), [issues]);
  const identifiers = useMemo(() => new Map(issues.map((i) => [i.id, i.identifier])), [issues]);
  if (issues.length === 0) return null;

  const run = (vars: Omit<IssueBatchVars, "ids">) => {
    onRequest(null);
    batch.mutate(
      { ids, ...vars },
      {
        onSuccess: (result) => {
          showToast(summarizeBatch(ids.length, result, (id) => identifiers.get(id) ?? id));
          if (result.failures.length === 0) onClear();
        },
        onError: (e) =>
          showToast({ message: errorMessage(e, "Could not update the issues."), tone: "error" }),
      },
    );
  };

  const me = session ? { id: session.user.id, name: session.user.name } : null;
  const states = meta.data?.states ?? [];
  const allLabels = meta.data?.labels ?? [];
  const usedLabels = new Set(issues.flatMap((i) => i.labelIds));
  const common = (kind: Exclude<BulkKind, "delete" | "team">) => ({
    open: request === kind,
    onOpenChange: (o: boolean) => !o && onRequest(null),
    title: TITLES[kind],
  });

  const barButton = (kind: BulkKind, label: string, disabled = false) => (
    <Button
      key={kind}
      variant="ghost"
      size="sm"
      disabled={disabled}
      onClick={() => onRequest(kind)}
      aria-label={`Bulk ${label}`}
    >
      {label}
    </Button>
  );
  const needsTeam = !single || !meta.data;
  // Teams ohne Estimates: keine Estimate-Aktion anbieten (B4.35).
  const estimatesOff = single && meta.data?.team.issueEstimationType === "notUsed";

  return (
    <>
      <div
        role="toolbar"
        aria-label="Bulk actions"
        className="pointer-events-auto fixed bottom-4 left-1/2 z-40 flex -translate-x-1/2 items-center gap-1 rounded-xl border border-border-strong bg-popover px-2 py-1.5 text-sm shadow-popover"
      >
        <span className="px-2 font-medium" aria-live="polite">
          {issues.length} selected
        </span>
        {barButton("status", "Status", needsTeam)}
        {barButton("priority", "Priority")}
        {barButton("assignee", "Assignee")}
        {barButton("labels", "Add label", needsTeam)}
        {barButton("labels-remove", "Remove label", needsTeam)}
        {barButton("project", "Project", needsTeam)}
        {barButton("cycle", "Cycle", needsTeam)}
        {!estimatesOff && barButton("estimate", "Estimate", needsTeam)}
        {barButton("team", "Move")}
        <Button
          variant="ghost"
          size="icon"
          aria-label="Bulk archive"
          title="Archive"
          onClick={() => run({ archive: true })}
        >
          <Archive />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Bulk delete"
          title="Delete"
          onClick={() => onRequest("delete")}
        >
          <Trash2 />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Clear selection"
          title="Clear selection (Esc)"
          onClick={onClear}
        >
          <X />
        </Button>
      </div>

      <PropertyPicker
        {...common("status")}
        options={statusOptions(states)}
        onSelect={(id) => run({ stateId: id })}
      />
      <PropertyPicker
        {...common("priority")}
        options={priorityOptions()}
        onSelect={(id) => run({ priority: Number(id) })}
      />
      <PropertyPicker
        {...common("assignee")}
        options={assigneeOptions(users, me)}
        onSelect={(id) => run({ assigneeId: id === NONE ? null : id })}
      />
      <PropertyPicker
        {...common("labels")}
        options={labelOptions(allLabels)}
        onSelect={(id) => run({ addedLabelIds: [id] })}
      />
      <PropertyPicker
        {...common("labels-remove")}
        options={labelOptions(allLabels.filter((l) => usedLabels.has(l.id)))}
        emptyText="The selected issues have no labels."
        onSelect={(id) => run({ removedLabelIds: [id] })}
      />
      <PropertyPicker
        {...common("estimate")}
        options={estimatePickerOptions(meta.data?.team.issueEstimationType ?? "notUsed")}
        onSelect={(id) => run({ estimate: id === NONE ? null : Number(id) })}
      />
      <PropertyPicker
        {...common("project")}
        options={projectOptions(meta.data?.projects ?? [])}
        onSelect={(id) => run({ projectId: id === NONE ? null : id })}
      />
      <PropertyPicker
        {...common("cycle")}
        options={cycleOptions(meta.data?.cycles ?? [])}
        onSelect={(id) => run({ cycleId: id === NONE ? null : id })}
      />
      <PropertyPicker
        open={request === "team" && pendingTeam === null}
        onOpenChange={(o) => !o && onRequest(null)}
        title="Move to team…"
        options={teamOptions(
          (teams.data ?? []).map((t) => ({ id: t.id, key: t.key, name: t.name })),
        )}
        emptyText={teams.isLoading ? "Loading teams…" : "No teams."}
        onSelect={(id) => {
          const t = (teams.data ?? []).find((x) => x.id === id);
          if (t) setPendingTeam({ id: t.id, name: t.name });
        }}
      />

      <Dialog
        open={pendingTeam !== null}
        onOpenChange={(o) => {
          if (!o) {
            setPendingTeam(null);
            onRequest(null);
          }
        }}
      >
        <DialogContent aria-describedby="bulk-move-desc" className="max-w-md">
          <div className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-base font-semibold">
              Move {issues.length} issue{issues.length === 1 ? "" : "s"} to {pendingTeam?.name}
            </DialogTitle>
            <DialogDescription id="bulk-move-desc" className="text-sm text-muted-foreground">
              Each issue gets a new identifier. Labels, projects and cycles that do not exist in the
              target team are removed.
            </DialogDescription>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={withSubs}
                onChange={(e) => setWithSubs(e.target.checked)}
              />
              Also move sub-issues
            </label>
            <div className="flex justify-end gap-2">
              <Button
                variant="ghost"
                onClick={() => {
                  setPendingTeam(null);
                  onRequest(null);
                }}
              >
                Cancel
              </Button>
              <Button
                onClick={() => {
                  const target = pendingTeam;
                  setPendingTeam(null);
                  if (target)
                    run({ teamId: target.id, ...(withSubs ? { moveSubIssues: true } : {}) });
                }}
              >
                Move issues
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={request === "delete"} onOpenChange={(o) => !o && onRequest(null)}>
        <DialogContent aria-describedby="bulk-delete-desc" className="max-w-md">
          <div className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-base font-semibold">
              Delete {issues.length} issue{issues.length === 1 ? "" : "s"}?
            </DialogTitle>
            <DialogDescription id="bulk-delete-desc" className="text-sm text-muted-foreground">
              The issues move to the trash and can be restored for 30 days.
            </DialogDescription>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => onRequest(null)}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={() => run({ delete: true })}>
                Delete
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
