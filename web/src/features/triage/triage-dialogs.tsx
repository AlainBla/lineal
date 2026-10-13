import { useState, type FormEvent, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { Issue, WorkflowState } from "@/gql/graphql";
import { PRIORITIES } from "@/features/issues/model";
import { fetchIssue } from "@/features/issues/queries";
import { errorMessage } from "@/features/issues/toast-store";
import { useShortcutContext } from "@/lib/shortcuts";

export type TriageDialogKind = "accept" | "decline" | "duplicate" | "snooze";

export interface TriageSubmit {
  kind: TriageDialogKind;
  stateId?: string;
  priority?: number;
  comment?: string;
  duplicateOfId?: string;
  snoozedUntilAt?: string;
}

const SELECT =
  "h-8 rounded-md border border-border-strong bg-transparent px-2 text-sm outline-none focus-visible:border-ring";

/** Zeitpunkte der Snooze-Voreinstellungen (lokale Zeit, 09:00). */
export function snoozePresets(
  now: Date = new Date(),
): Array<{ id: string; label: string; at: Date }> {
  const at = (days: number) => {
    const d = new Date(now);
    d.setDate(d.getDate() + days);
    d.setHours(9, 0, 0, 0);
    return d;
  };
  const untilMonday = (8 - now.getDay()) % 7 || 7;
  return [
    { id: "tomorrow", label: "Tomorrow", at: at(1) },
    { id: "next-week", label: "Next week", at: at(untilMonday) },
  ];
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs text-muted-foreground">
      {label}
      {children}
    </label>
  );
}

const TITLE: Record<TriageDialogKind, string> = {
  accept: "Accept",
  decline: "Decline",
  duplicate: "Mark as duplicate of…",
  snooze: "Snooze",
};

/**
 * Popover der Triage-Aktionen (docs/08 §2.5): Accept (Status, Priority, Kommentar), Decline
 * (Kommentar), Duplicate (Ziel-Issue) und Snooze (Zeitpunkt).
 */
export function TriageDialog({
  kind,
  issue,
  states,
  onClose,
  onSubmit,
}: {
  kind: TriageDialogKind | null;
  issue: Issue | null;
  states: WorkflowState[];
  onClose: () => void;
  onSubmit: (v: TriageSubmit) => Promise<void>;
}) {
  useShortcutContext("modal", kind !== null);
  return (
    <Dialog open={kind !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        {kind && issue && (
          <TriageForm
            key={`${kind}:${issue.id}`}
            kind={kind}
            issue={issue}
            states={states}
            onClose={onClose}
            onSubmit={onSubmit}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function TriageForm({
  kind,
  issue,
  states,
  onClose,
  onSubmit,
}: {
  kind: TriageDialogKind;
  issue: Issue;
  states: WorkflowState[];
  onClose: () => void;
  onSubmit: (v: TriageSubmit) => Promise<void>;
}) {
  const [comment, setComment] = useState("");
  const [stateId, setStateId] = useState("");
  const [priority, setPriority] = useState("");
  const [ref, setRef] = useState("");
  const [preset, setPreset] = useState("tomorrow");
  const [custom, setCustom] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const acceptStates = states.filter((s) => ["backlog", "unstarted", "started"].includes(s.type));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const text = comment.trim() === "" ? undefined : comment.trim();
      if (kind === "accept") {
        await onSubmit({
          kind,
          stateId: stateId || undefined,
          priority: priority === "" ? undefined : Number(priority),
          comment: text,
        });
      } else if (kind === "decline") {
        await onSubmit({ kind, comment: text });
      } else if (kind === "duplicate") {
        const target = ref.trim();
        if (!target) {
          setError("Enter an issue identifier such as ENG-12.");
          return;
        }
        const other = await fetchIssue(target);
        if (other.id === issue.id) {
          setError("An issue cannot be a duplicate of itself.");
          return;
        }
        await onSubmit({ kind, duplicateOfId: other.id, comment: text });
      } else {
        let at: Date | undefined;
        if (preset === "custom") {
          at = custom ? new Date(custom) : undefined;
          if (!at || Number.isNaN(at.getTime())) {
            setError("Pick a date and time.");
            return;
          }
        } else {
          at = snoozePresets().find((p) => p.id === preset)?.at;
        }
        if (!at) return;
        await onSubmit({ kind, snoozedUntilAt: at.toISOString() });
      }
      onClose();
    } catch (err) {
      setError(errorMessage(err, "The triage action failed."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-3 p-4">
      <DialogTitle className="text-base font-semibold">
        {TITLE[kind]} {issue.identifier}
      </DialogTitle>
      <DialogDescription className="sr-only">
        Triage action for {issue.identifier}.
      </DialogDescription>
      {kind === "accept" && (
        <>
          <Field label="Status">
            <select
              aria-label="Status"
              className={SELECT}
              value={stateId}
              onChange={(e) => setStateId(e.target.value)}
            >
              <option value="">Default status</option>
              {acceptStates.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Priority">
            <select
              aria-label="Priority"
              className={SELECT}
              value={priority}
              onChange={(e) => setPriority(e.target.value)}
            >
              <option value="">{issue.priority === 0 ? "No priority" : "Keep priority"}</option>
              {PRIORITIES.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </Field>
        </>
      )}
      {kind === "duplicate" && (
        <Input
          autoFocus
          aria-label="Issue identifier"
          placeholder="Duplicate of, e.g. ENG-12"
          value={ref}
          onChange={(e) => setRef(e.target.value)}
        />
      )}
      {kind === "snooze" && (
        <div role="radiogroup" aria-label="Snooze until" className="flex flex-col gap-1.5">
          {[
            ...snoozePresets().map((p) => ({
              id: p.id,
              label: `${p.label} (${p.at.toLocaleDateString()})`,
            })),
            { id: "custom", label: "Custom…" },
          ].map((p) => (
            <label key={p.id} className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="snooze"
                checked={preset === p.id}
                onChange={() => setPreset(p.id)}
              />
              {p.label}
            </label>
          ))}
          {preset === "custom" && (
            <Input
              type="datetime-local"
              aria-label="Snooze until date and time"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
            />
          )}
        </div>
      )}
      {kind !== "snooze" && (
        <Field label={kind === "decline" ? "Reason (optional)" : "Comment (optional)"}>
          <textarea
            aria-label="Comment"
            rows={3}
            className="rounded-md border border-border-strong bg-transparent p-2 text-sm text-foreground outline-none focus-visible:border-ring"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
        </Field>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          type="submit"
          disabled={busy}
          variant={kind === "decline" ? "destructive" : "default"}
        >
          {kind === "snooze" ? "Snooze" : kind === "duplicate" ? "Mark as duplicate" : TITLE[kind]}
        </Button>
      </div>
    </form>
  );
}
