/** Team-Zugriff (Sichtbarkeit, Beitritt) und „Danger zone“ (stilllegen, reaktivieren, löschen). */
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";

import { useSession } from "@/app/auth";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { errorText } from "@/features/issues/error-text";
import type { Team } from "@/gql/graphql";

import { useUpdateTeam } from "./queries";
import { useTeamLifecycle, useTeamsOverview } from "./teams-queries";
import { allowedVisibilities } from "./team-key";
import { Section, selectClass } from "./ui";

const VISIBILITY_LABEL: Record<string, string> = {
  public: "Public – everyone in the workspace",
  private: "Private – members only",
  restricted: "Restricted – members and members of the parent team",
};

/** Sichtbarkeit und Beitrittsregel (docs/02 §5, §4.2). Die Rechte prüft der Server. */
export function TeamAccessSection({ team }: { team: Team }) {
  const update = useUpdateTeam(team.id);
  const overview = useTeamsOverview();
  const [visibility, setVisibility] = useState(team.visibility);
  const [canJoin, setCanJoin] = useState(team.allMembersCanJoin);
  const parent = team.parentId ? overview.data?.find((t) => t.id === team.parentId) : undefined;
  const options = Array.from(new Set([team.visibility, ...allowedVisibilities(parent)]));
  const dirty = visibility !== team.visibility || canJoin !== team.allMembersCanJoin;
  return (
    <Section title="Access" description="Who can see this team and who can join it.">
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          update.mutate({
            ...(visibility !== team.visibility ? { visibility } : {}),
            ...(canJoin !== team.allMembersCanJoin ? { allMembersCanJoin: canJoin } : {}),
          });
        }}
      >
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-xs text-muted-foreground">Visibility</span>
          <select
            aria-label="Team visibility"
            className={selectClass}
            value={visibility}
            onChange={(e) => setVisibility(e.target.value)}
          >
            {options.map((v) => (
              <option key={v} value={v}>
                {VISIBILITY_LABEL[v] ?? v}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            aria-label="All members can join"
            checked={canJoin}
            onChange={(e) => setCanJoin(e.target.checked)}
          />
          All workspace members can join this team
        </label>
        <div className="flex items-center gap-3">
          <Button type="submit" size="sm" disabled={!dirty || update.isPending}>
            Save
          </Button>
          {update.isError && (
            <span role="alert" className="text-xs text-destructive">
              {errorText(update.error)}
            </span>
          )}
        </div>
      </form>
    </Section>
  );
}

type Handling = "" | "retire" | "unnest" | "delete";

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  children,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3 p-4">{children}</div>
      </DialogContent>
    </Dialog>
  );
}

/** „Danger zone“: Team stilllegen / reaktivieren / löschen (docs/02 §2.7). */
export function TeamDangerZone({ team }: { team: Team }) {
  const session = useSession();
  const navigate = useNavigate();
  const overview = useTeamsOverview();
  const { retire, unretire, remove } = useTeamLifecycle(team.id);
  const retired = !!team.retiredAt;
  const children = (overview.data ?? []).filter((t) => t.parentId === team.id);
  const activeChildren = children.filter((t) => !t.retiredAt);
  const [dialog, setDialog] = useState<"retire" | "delete" | null>(null);
  const [handling, setHandling] = useState<Handling>("");
  const [typed, setTyped] = useState("");
  const ws = session?.workspace.urlKey ?? "";

  const close = () => {
    setDialog(null);
    setHandling("");
    setTyped("");
    retire.reset();
    remove.reset();
  };
  const needsRetireChoice = activeChildren.length > 0;
  const needsDeleteChoice = children.length > 0;

  return (
    <Section
      title="Danger zone"
      description="Retiring makes the team read-only. Deleting moves it to the trash."
    >
      <div className="flex flex-col divide-y divide-border rounded-lg border border-destructive/40">
        <div className="flex items-center gap-3 px-3 py-2 text-sm">
          <div className="flex-1">
            <p className="font-medium">{retired ? "Reactivate team" : "Retire team"}</p>
            <p className="text-xs text-muted-foreground">
              {retired
                ? "This team is retired and read-only. Reactivate it to work in it again."
                : "The team and its issues become read-only and disappear from the sidebar."}
            </p>
          </div>
          {retired ? (
            <Button
              variant="outline"
              size="sm"
              disabled={unretire.isPending}
              onClick={() => unretire.mutate()}
            >
              Reactivate team
            </Button>
          ) : (
            <Button variant="outline" size="sm" onClick={() => setDialog("retire")}>
              Retire team…
            </Button>
          )}
        </div>
        {unretire.isError && (
          <p role="alert" className="px-3 py-2 text-xs text-destructive">
            {errorText(unretire.error)}
          </p>
        )}
        <div className="flex items-center gap-3 px-3 py-2 text-sm">
          <div className="flex-1">
            <p className="font-medium">Delete team</p>
            <p className="text-xs text-muted-foreground">
              Issues, statuses and labels of this team are removed after the retention period.
            </p>
          </div>
          <Button variant="destructive" size="sm" onClick={() => setDialog("delete")}>
            Delete team…
          </Button>
        </div>
      </div>

      <ConfirmDialog
        open={dialog === "retire"}
        onOpenChange={(o) => !o && close()}
        title={`Retire ${team.name}?`}
        description="The team becomes read-only: no new issues or members."
      >
        {needsRetireChoice && (
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-xs text-muted-foreground">
              This team has {activeChildren.length} active sub-team
              {activeChildren.length === 1 ? "" : "s"}
            </span>
            <select
              aria-label="Sub-teams"
              className={selectClass}
              value={handling}
              onChange={(e) => setHandling(e.target.value as Handling)}
            >
              <option value="">Choose what happens to them…</option>
              <option value="retire">Retire them too</option>
              <option value="unnest">Make them root teams</option>
            </select>
          </label>
        )}
        <div className="flex items-center gap-3">
          <Button
            size="sm"
            disabled={retire.isPending || (needsRetireChoice && handling === "")}
            onClick={() =>
              retire.mutate(needsRetireChoice ? (handling as "retire" | "unnest") : undefined, {
                onSuccess: close,
              })
            }
          >
            Retire team
          </Button>
          <Button variant="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          {retire.isError && (
            <span role="alert" className="text-xs text-destructive">
              {errorText(retire.error)}
            </span>
          )}
        </div>
      </ConfirmDialog>

      <ConfirmDialog
        open={dialog === "delete"}
        onOpenChange={(o) => !o && close()}
        title={`Delete ${team.name}?`}
        description="The team and its issues disappear for everyone. This can only be undone by an admin within the retention period."
      >
        {needsDeleteChoice && (
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-xs text-muted-foreground">
              This team has {children.length} sub-team{children.length === 1 ? "" : "s"}
            </span>
            <select
              aria-label="Sub-teams"
              className={selectClass}
              value={handling}
              onChange={(e) => setHandling(e.target.value as Handling)}
            >
              <option value="">Choose what happens to them…</option>
              <option value="delete">Delete them too</option>
              <option value="unnest">Make them root teams</option>
            </select>
          </label>
        )}
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-xs text-muted-foreground">
            Type the team identifier <strong>{team.key}</strong> to confirm
          </span>
          <Input
            aria-label="Confirm team identifier"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
          />
        </label>
        <div className="flex items-center gap-3">
          <Button
            variant="destructive"
            size="sm"
            disabled={
              remove.isPending || typed !== team.key || (needsDeleteChoice && handling === "")
            }
            onClick={() =>
              remove.mutate(needsDeleteChoice ? (handling as "delete" | "unnest") : undefined, {
                onSuccess: () => {
                  close();
                  void navigate({ to: `/${ws}/settings/teams` as "/" });
                },
              })
            }
          >
            Delete team
          </Button>
          <Button variant="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          {remove.isError && (
            <span role="alert" className="text-xs text-destructive">
              {errorText(remove.error)}
            </span>
          )}
        </div>
      </ConfirmDialog>
    </Section>
  );
}
