/**
 * Label zusammenführen oder endgültig löschen (docs/03 §5.4): zeigt vorab, wie viele Issues
 * betroffen sind, und verlangt eine ausdrückliche Bestätigung. Eigener Dialog, damit der
 * Label-Manager nur zwei Schaltflächen und einen Dialogaufruf braucht.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { errorText } from "@/features/issues/error-text";
import type { IssueLabel } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { selectClass } from "./ui";

export type LabelBulkMode = "merge" | "delete";

const done = (qc: ReturnType<typeof useQueryClient>) => {
  void qc.invalidateQueries({ queryKey: ["settings"] });
  void qc.invalidateQueries({ queryKey: ["team-meta"] });
  void qc.invalidateQueries({ queryKey: ["issues"] });
  void qc.invalidateQueries({ queryKey: ["issue"] });
};

function useUsage(id: string) {
  return useQuery({
    queryKey: ["settings", "label-usage", id],
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ issueLabelUsage: number }, { id: string }>(
          `query LabelUsage($id: String!) { issueLabelUsage(id: $id) }`,
          { id },
          { signal },
        )
      ).issueLabelUsage,
  });
}

/** Mögliche Merge-Ziele: Workspace-Labels bzw. (bei Team-Labels) zusätzlich Labels des Teams. */
function useCandidates(label: IssueLabel, enabled: boolean) {
  return useQuery({
    queryKey: ["settings", "label-merge-targets", label.id],
    enabled,
    queryFn: async ({ signal }) => {
      const res = await gqlRequest<{ issueLabels: IssueLabel[] }, { teamId: string | null }>(
        `query LabelMergeTargets($teamId: String) {
          issueLabels(teamId: $teamId, includeGroups: false) { id name color teamId parentId isGroup retiredAt }
        }`,
        { teamId: label.teamId ?? null },
        { signal },
      );
      return res.issueLabels.filter(
        (l) =>
          l.id !== label.id &&
          !l.isGroup &&
          !l.retiredAt &&
          (label.teamId ? l.teamId === null || l.teamId === label.teamId : l.teamId === null),
      );
    },
  });
}

export function LabelBulkDialog({
  label,
  mode,
  onClose,
}: {
  label: IssueLabel;
  mode: LabelBulkMode;
  onClose: () => void;
}) {
  useShortcutContext("modal", true);
  const qc = useQueryClient();
  const usage = useUsage(label.id);
  const candidates = useCandidates(label, mode === "merge");
  const [target, setTarget] = useState("");
  const merge = useMutation({
    mutationFn: (targetId: string) =>
      gqlRequest(
        `mutation LabelMerge($id: String!, $targetId: String!) {
          issueLabelMerge(id: $id, targetId: $targetId) { success affectedIssues }
        }`,
        { id: label.id, targetId },
      ),
    onSuccess: () => {
      done(qc);
      onClose();
    },
  });
  const del = useMutation({
    mutationFn: () =>
      gqlRequest(
        `mutation LabelDelete($id: String!) { issueLabelDelete(id: $id) { success affectedIssues } }`,
        { id: label.id },
      ),
    onSuccess: () => {
      done(qc);
      onClose();
    },
  });
  const busy = merge.isPending || del.isPending;
  const count = usage.data;
  const issues = count === undefined ? "…" : `${count} issue${count === 1 ? "" : "s"}`;
  const error = merge.error ?? del.error ?? usage.error;

  return (
    <Dialog open onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent aria-describedby="label-bulk-desc" className="max-w-md">
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">
            {mode === "merge" ? `Merge “${label.name}”` : `Delete “${label.name}”`}
          </DialogTitle>
          <DialogDescription id="label-bulk-desc" className="text-sm text-muted-foreground">
            {mode === "merge"
              ? `All ${issues} with this label, and every template and view that uses it, switch to the label you pick. “${label.name}” is deleted afterwards.`
              : `${label.name} is used by ${issues}. It is removed from all of them and deleted permanently${label.isGroup ? ", together with all labels in the group" : ""}. This cannot be undone.`}
          </DialogDescription>
          {mode === "merge" && (
            <label className="flex flex-col gap-1 text-sm">
              Merge into
              <select
                aria-label="Merge into"
                className={selectClass}
                value={target}
                onChange={(e) => setTarget(e.target.value)}
              >
                <option value="">Choose a label…</option>
                {(candidates.data ?? []).map((l) => (
                  <option key={l.id} value={l.id}>
                    {l.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {error != null && (
            <p role="alert" className="text-xs text-destructive">
              {errorText(error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            {mode === "merge" ? (
              <Button disabled={!target || busy} onClick={() => merge.mutate(target)}>
                Merge label
              </Button>
            ) : (
              <Button
                variant="destructive"
                disabled={busy || usage.isLoading}
                onClick={() => del.mutate()}
              >
                Delete label
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
