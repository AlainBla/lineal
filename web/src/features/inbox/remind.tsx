/**
 * „Remind me“ (F.10, docs/05 §6.10): Zeitpunkt für Issue, Projekt, Initiative oder Dokument wählen.
 * Ein globaler Host (`RemindHost`, in der Shell) zeigt den Dialog; Aufrufer (Issue-Kürzel `⇧ H`,
 * Projekt-, Initiativen- und Dokumentseiten, Command-Menü) rufen nur `requestRemind`.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, useSyncExternalStore } from "react";

import { useActions } from "@/app/commands";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { showToast } from "@/features/issues/toast-store";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { snoozePresets } from "./queries";

export type RemindTargetType = "issue" | "project" | "initiative" | "document";

export interface RemindTarget {
  targetType: RemindTargetType;
  /** Issue: Kennung oder UUID; sonst UUID. */
  target: string;
  /** Anzeigename im Dialog (Titel/Kennung). */
  label?: string;
}

let current: RemindTarget | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function requestRemind(t: RemindTarget | null) {
  current = t;
  emit();
}

function useRemindTarget(): RemindTarget | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
    () => current,
  );
}

export const remindKeys = {
  one: (t: RemindTarget) => ["reminder", t.targetType, t.target] as const,
};

interface ReminderRow {
  id: string;
  targetType: string;
  targetId: string;
  remindAt: string;
}

export async function fetchReminder(t: RemindTarget): Promise<ReminderRow | null> {
  const res = await gqlRequest<
    { reminder: ReminderRow | null },
    { targetType: string; targetId: string }
  >(
    `query MyReminder($targetType: String!, $targetId: String!) {
      reminder(targetType: $targetType, targetId: $targetId) { id targetType targetId remindAt }
    }`,
    { targetType: t.targetType, targetId: t.target },
  );
  return res.reminder;
}

export async function setReminder(t: RemindTarget, remindAt: string | null): Promise<void> {
  await gqlRequest(
    `mutation ReminderSet($targetType: String!, $targetId: String!, $remindAt: String) {
      reminderSet(targetType: $targetType, targetId: $targetId, remindAt: $remindAt) {
        success reminder { id remindAt }
      }
    }`,
    { targetType: t.targetType, targetId: t.target, remindAt },
  );
}

const fmt = (iso: string) =>
  new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** Dialog „Remind me…“ mit den Zeitvorgaben wie bei Snooze (docs/08 §2.9) und „Custom…“. */
export function RemindHost() {
  const t = useRemindTarget();
  const open = t !== null;
  useShortcutContext("modal", open);
  const qc = useQueryClient();
  const existing = useQuery({
    queryKey: t ? remindKeys.one(t) : ["reminder", "none"],
    queryFn: () => fetchReminder(t!),
    enabled: open,
  });
  const [custom, setCustom] = useState("");
  const mut = useMutation({
    mutationFn: (v: { t: RemindTarget; at: string | null }) => setReminder(v.t, v.at),
    onSuccess: (_d, v) => {
      void qc.invalidateQueries({ queryKey: remindKeys.one(v.t) });
      showToast({
        message: v.at ? `Reminder set for ${fmt(v.at)}` : "Reminder removed",
        onUndo: v.at ? () => mut.mutate({ t: v.t, at: null }) : undefined,
      });
      requestRemind(null);
    },
    onError: (e) =>
      showToast({
        message: e instanceof Error ? e.message : "Could not set the reminder",
        tone: "error",
      }),
  });
  const customDate = custom ? new Date(custom) : null;
  const customValid = !!customDate && !Number.isNaN(customDate.getTime());
  return (
    <Dialog open={open} onOpenChange={(o) => !o && requestRemind(null)}>
      <DialogContent className="max-w-sm">
        <div className="flex flex-col gap-2 p-4">
          <DialogTitle className="text-sm font-medium">
            Remind me{t?.label ? ` · ${t.label}` : ""}
          </DialogTitle>
          <DialogDescription className="text-xs text-muted-foreground">
            {existing.data
              ? `Current reminder: ${fmt(existing.data.remindAt)}`
              : "You get a notification in your inbox at the chosen time."}
          </DialogDescription>
          <div role="menu" aria-label="Remind me" className="flex flex-col">
            {snoozePresets(new Date()).map((p) => (
              <button
                key={p.id}
                type="button"
                role="menuitem"
                className="rounded-md px-2 py-1.5 text-left text-sm hover:bg-surface-hover focus-visible:bg-surface-hover"
                onClick={() => t && mut.mutate({ t, at: p.until.toISOString() })}
              >
                {p.label}
              </button>
            ))}
          </div>
          <form
            className="flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (!t || !customValid || !customDate) return;
              if (customDate.getTime() <= Date.now()) {
                showToast({ message: "Pick a time in the future", tone: "error" });
                return;
              }
              mut.mutate({ t, at: customDate.toISOString() });
            }}
          >
            <input
              type="datetime-local"
              aria-label="Remind at"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              className="h-8 flex-1 rounded-md border border-input bg-transparent px-2 text-sm"
            />
            <Button type="submit" size="sm" disabled={!customValid}>
              Set
            </Button>
          </form>
          {existing.data && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => t && mut.mutate({ t, at: null })}
              className="self-start"
            >
              Remove reminder
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Registriert "Remind me..." (Shift+H) fuer das Objekt der aktuellen Seite (nur wenn `t` gesetzt ist). */
export function useRemindAction(t: RemindTarget | null, context?: "issue" | "list"): void {
  useActions([
    {
      id: `remind.${t?.targetType ?? "none"}`,
      title: "Remind me…",
      section: t?.targetType === "issue" ? "Issues" : "Actions",
      keywords: ["reminder", "snooze"],
      keys: "shift+h",
      context,
      enabled: t !== null,
      perform: () => t && requestRemind(t),
    },
  ]);
}
