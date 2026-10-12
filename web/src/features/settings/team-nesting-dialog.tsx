/**
 * Nesting-Wizard (docs/02 §3.3): Parent wählen oder lösen, Konflikte der Vorschau ansehen
 * (Mitglieder, Sichtbarkeit, Status, Labels, Cycles, Schätzung), Optionen wählen, ausführen.
 */
import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import { showToast } from "@/features/issues/toast-store";
import { useShortcutContext } from "@/lib/shortcuts";

import {
  parentCandidates,
  useNestTeam,
  useParentStates,
  usePreviewNesting,
  type NestingConflict,
  type StateMappingEntry,
} from "./team-nesting-queries";
import type { TeamRow } from "./teams-queries";
import { Field, selectClass } from "./ui";

function Check({
  label,
  checked,
  onChange,
  hint,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  hint?: string;
}) {
  return (
    <label className="flex items-start gap-2 text-sm">
      <input
        type="checkbox"
        className="mt-1"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>
        {label}
        {hint && <span className="block text-xs text-muted-foreground">{hint}</span>}
      </span>
    </label>
  );
}

function ConflictBox({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section
      aria-label={title}
      className="flex flex-col gap-2 rounded-md border border-border p-3 text-sm"
    >
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {title}
      </h3>
      {children}
    </section>
  );
}

const find = (conflicts: NestingConflict[], kind: string) => conflicts.find((c) => c.kind === kind);

function Preview({
  team,
  parent,
  conflicts,
  onDone,
  onBack,
}: {
  team: TeamRow;
  parent: TeamRow;
  conflicts: NestingConflict[];
  onDone: () => void;
  onBack: () => void;
}) {
  const nest = useNestTeam();
  const parentStates = useParentStates(parent.id);
  const members = find(conflicts, "membership");
  const visibility = find(conflicts, "visibility");
  const states = find(conflicts, "workflowState");
  const labels = find(conflicts, "label");
  const cycle = find(conflicts, "cycle");
  const estimate = find(conflicts, "estimate");
  const entries = useMemo(() => (states?.details.states ?? []) as StateMappingEntry[], [states]);
  const [addMembers, setAddMembers] = useState(true);
  const [downgrade, setDowngrade] = useState(false);
  const [inheritStates, setInheritStates] = useState(true);
  const [inheritEstimate, setInheritEstimate] = useState(true);
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const memberIds = (members?.details.userIds ?? []) as string[];
  const visTeams = (visibility?.details.teams ?? []) as Array<{
    teamId: string;
    from: string;
    to: string;
  }>;
  const blocked = (!!visibility && !downgrade) || (!!members && !addMembers);

  const apply = () => {
    // Nur Abweichungen von der automatischen Zuordnung senden.
    const stateMapping = entries
      .filter((e) => mapping[e.id] && mapping[e.id] !== e.targetId)
      .map((e) => ({ fromStateId: e.id, toStateId: mapping[e.id]! }));
    nest.mutate(
      {
        teamId: team.id,
        parentId: parent.id,
        options: {
          addMissingMembersToParent: addMembers,
          downgradeVisibility: downgrade,
          inheritWorkflowStatuses: inheritStates,
          inheritIssueEstimation: inheritEstimate,
          stateMapping: inheritStates ? stateMapping : [],
        },
      },
      {
        onSuccess: () => {
          showToast({ message: `${team.name} moved under ${parent.name}` });
          onDone();
        },
      },
    );
  };

  return (
    <div className="flex flex-col gap-3">
      {conflicts.length === 0 && (
        <p className="text-sm text-muted-foreground">No conflicts. The team can be moved as is.</p>
      )}
      {members && (
        <ConflictBox title="Members">
          <Check
            label={`Add ${memberIds.length} missing member${memberIds.length === 1 ? "" : "s"} to ${parent.name} and its parents`}
            hint="Members of the team (and its sub-teams) must also belong to the parent team."
            checked={addMembers}
            onChange={setAddMembers}
          />
        </ConflictBox>
      )}
      {visibility && (
        <ConflictBox title="Visibility">
          <p className="text-muted-foreground">{visibility.message}</p>
          <Check
            label={`Downgrade ${visTeams.length} team${visTeams.length === 1 ? "" : "s"} to restricted`}
            checked={downgrade}
            onChange={setDowngrade}
          />
        </ConflictBox>
      )}
      {states && (
        <ConflictBox title="Workflow statuses">
          <Check
            label={`Use the workflow statuses of ${parent.name}`}
            hint="Issues move to the matching parent status. Change the target where the automatic match is wrong."
            checked={inheritStates}
            onChange={setInheritStates}
          />
          {inheritStates && (
            <ul aria-label="Status mapping" className="flex flex-col gap-1">
              {entries.map((e) => (
                <li key={e.id} className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate">
                    {e.name} <span className="text-xs text-muted-foreground">({e.type})</span>
                  </span>
                  <span aria-hidden>→</span>
                  <select
                    aria-label={`Target for ${e.name}`}
                    className={selectClass}
                    value={mapping[e.id] ?? e.targetId}
                    onChange={(ev) => setMapping((m) => ({ ...m, [e.id]: ev.target.value }))}
                  >
                    {(
                      parentStates.data ?? [{ id: e.targetId, name: e.targetName, type: e.type }]
                    ).map((s) => (
                      <option key={s.id} value={s.id}>
                        {s.name}
                      </option>
                    ))}
                  </select>
                </li>
              ))}
            </ul>
          )}
        </ConflictBox>
      )}
      {labels && (
        <ConflictBox title="Labels">
          <p className="text-muted-foreground">{labels.message}</p>
          <ul aria-label="Duplicate labels" className="list-disc pl-5">
            {((labels.details.labels ?? []) as Array<{ id: string; name: string }>).map((l) => (
              <li key={l.id}>{l.name}</li>
            ))}
          </ul>
        </ConflictBox>
      )}
      {cycle && (
        <ConflictBox title="Cycles">
          <p className="text-muted-foreground">{cycle.message}</p>
          <p className="text-xs text-muted-foreground">
            Own upcoming cycles replaced: {String(cycle.details.upcomingCycles ?? 0)}
          </p>
        </ConflictBox>
      )}
      {estimate && (
        <ConflictBox title="Estimates">
          <Check
            label={`Use the estimation scale of ${parent.name} (${String(estimate.details.from)} → ${String(estimate.details.to)})`}
            checked={inheritEstimate}
            onChange={setInheritEstimate}
          />
        </ConflictBox>
      )}
      {nest.error != null && (
        <p role="alert" className="text-xs text-destructive">
          {errorText(nest.error)}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onBack}>
          Back
        </Button>
        <Button type="button" disabled={nest.isPending || blocked} onClick={apply}>
          Move team
        </Button>
      </div>
    </div>
  );
}

function Wizard({ team, all, onClose }: { team: TeamRow; all: TeamRow[]; onClose: () => void }) {
  const candidates = useMemo(() => parentCandidates(team, all), [team, all]);
  const [parentId, setParentId] = useState<string>(team.parentId ?? "");
  const [step, setStep] = useState<"choose" | "preview">("choose");
  const nest = useNestTeam();
  const target = candidates.find((t) => t.id === parentId) ?? null;
  const unchanged = parentId === (team.parentId ?? "");
  const preview = usePreviewNesting(team.id, parentId || null, step === "preview" && !!target);

  const unnest = () =>
    nest.mutate(
      { teamId: team.id, parentId: null },
      {
        onSuccess: () => {
          showToast({ message: `${team.name} is now a top-level team` });
          onClose();
        },
      },
    );

  return (
    <div className="flex flex-col gap-3 p-4">
      <DialogHeader>
        <DialogTitle className="text-base font-semibold">Move {team.name}</DialogTitle>
        <DialogDescription className="text-sm text-muted-foreground">
          Put the team under another team or make it a top-level team.
        </DialogDescription>
      </DialogHeader>
      {step === "choose" && (
        <>
          <Field label="Parent team">
            <select
              aria-label="Parent team"
              className={selectClass}
              value={parentId}
              onChange={(e) => setParentId(e.target.value)}
            >
              <option value="">No parent (top level)</option>
              {candidates.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name} ({t.key})
                </option>
              ))}
            </select>
          </Field>
          {nest.error != null && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(nest.error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            {target ? (
              <Button type="button" disabled={unchanged} onClick={() => setStep("preview")}>
                Review changes
              </Button>
            ) : (
              <Button type="button" disabled={unchanged || nest.isPending} onClick={unnest}>
                Make top-level
              </Button>
            )}
          </div>
        </>
      )}
      {step === "preview" && target && (
        <>
          {preview.isLoading && <p className="text-sm text-muted-foreground">Checking…</p>}
          {preview.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(preview.error)}
            </p>
          )}
          {preview.data && (
            <Preview
              team={team}
              parent={target}
              conflicts={preview.data}
              onDone={onClose}
              onBack={() => setStep("choose")}
            />
          )}
          {preview.isError && (
            <div className="flex justify-end">
              <Button type="button" variant="ghost" onClick={() => setStep("choose")}>
                Back
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

/** Dialog „Move team“; `team = null` schließt ihn. Der Wizard setzt sich je Team zurück (`key`). */
export function TeamNestingDialog({
  team,
  all,
  onClose,
}: {
  team: TeamRow | null;
  all: TeamRow[];
  onClose: () => void;
}) {
  useShortcutContext("modal", !!team);
  return (
    <Dialog open={!!team} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        {team && <Wizard key={team.id} team={team} all={all} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  );
}
