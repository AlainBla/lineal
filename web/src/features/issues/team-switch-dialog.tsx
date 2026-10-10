import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { useTeamMeta, useTeams } from "./hooks";
import { useIssueActions } from "./issue-actions";
import { previewMove } from "./model";
import { PropertyPicker } from "./picker";
import { teamOptions } from "./property-options";
import { fetchTeamMeta, keys, type TeamRef } from "./queries";
import { errorMessage, showToast } from "./toast-store";

/**
 * Teamwechsel-Dialog (`Ctrl ⇧ M`, docs/08 F7): Team wählen, bei Verlusten bestätigen, verschieben,
 * Toast mit Undo und Wechsel auf die neue Kennung.
 */
export function TeamSwitchDialog({ issue, onClose }: { issue: Issue; onClose: () => void }) {
  const teams = useTeams();
  const source = useTeamMeta(issue.teamId);
  const actions = useIssueActions();
  const session = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [pending, setPending] = useState<{ target: TeamRef; losses: string[] } | null>(null);
  const [busy, setBusy] = useState(false);
  useShortcutContext("modal", pending !== null);

  const list = (teams.data ?? []).map((t) => ({ id: t.id, key: t.key, name: t.name }));
  const options = teamOptions(list).map((o) =>
    o.id === issue.teamId ? { ...o, hint: "Current" } : o,
  );

  const execute = async (target: TeamRef) => {
    setBusy(true);
    const oldTeamId = issue.teamId;
    const oldIdentifier = issue.identifier;
    const moved = await actions.moveToTeam(issue, target.id);
    setBusy(false);
    if (!moved) return;
    setPending(null);
    onClose();
    const ws = session?.workspace.urlKey ?? "";
    showToast({
      message: `Moved to ${moved.identifier}`,
      href: `/${ws}/issue/${moved.identifier}`,
      onUndo: () => {
        void actions.moveToTeam(moved, oldTeamId).then((back) => {
          if (back)
            void navigate({ to: `/${ws}/issue/${back.identifier || oldIdentifier}` as "/" });
        });
      },
    });
    void navigate({ to: `/${ws}/issue/${moved.identifier}` as "/", replace: true });
  };

  const choose = async (teamId: string) => {
    const target = (teams.data ?? []).find((t) => t.id === teamId);
    if (!target || target.id === issue.teamId) {
      onClose();
      return;
    }
    try {
      const targetMeta = await qc.fetchQuery({
        queryKey: keys.teamMeta(target.id),
        queryFn: ({ signal }) => fetchTeamMeta(target.id, signal),
        staleTime: 60_000,
      });
      const { losses } = previewMove(issue, {
        labels: targetMeta.labels,
        projects: targetMeta.projects,
        estimationType: targetMeta.team.issueEstimationType,
        sourceEstimationType: source.data?.team.issueEstimationType ?? "notUsed",
      });
      if (losses.length === 0) await execute(target);
      else setPending({ target, losses });
    } catch (e) {
      showToast({ message: errorMessage(e, "Could not load the team."), tone: "error" });
      onClose();
    }
  };

  return (
    <>
      <PropertyPicker
        open={pending === null}
        onOpenChange={(o) => !o && onClose()}
        title="Move to team…"
        options={options}
        selected={issue.teamId}
        onSelect={(id) => void choose(id)}
        emptyText={teams.isLoading ? "Loading teams…" : "No teams."}
      />
      <Dialog open={pending !== null} onOpenChange={(o) => !o && !busy && onClose()}>
        <DialogContent aria-describedby="move-desc" className="max-w-md">
          <div className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-base font-semibold">
              Move issue to {pending?.target.name}
            </DialogTitle>
            <DialogDescription id="move-desc" className="text-sm text-muted-foreground">
              {issue.identifier} will get a new identifier in {pending?.target.key}.
            </DialogDescription>
            <ul className="list-disc pl-5 text-sm">
              {pending?.losses.map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={onClose} disabled={busy}>
                Cancel
              </Button>
              <Button onClick={() => pending && void execute(pending.target)} disabled={busy}>
                Move issue
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
