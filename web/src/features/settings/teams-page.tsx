import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useMemo, useState } from "react";

import { useSession } from "@/app/auth";
import { setUiState } from "@/app/ui-state";
import { Button } from "@/components/ui/button";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";

import { workspaceSettingsNav } from "./nav";
import { TeamNestingDialog } from "./team-nesting-dialog";
import { DeletedTeamsSection } from "./teams-deleted";
import { useJoinTeam, useTeamsOverview, type TeamRow } from "./teams-queries";
import { Section, SettingsShell } from "./ui";

/** Teams in Baumreihenfolge (Parent, dann Sub-Teams); fehlt der Parent, steht das Team an der Wurzel. */
export function orderTeams(teams: TeamRow[]): Array<{ team: TeamRow; depth: number }> {
  const ids = new Set(teams.map((t) => t.id));
  const byParent = new Map<string | null, TeamRow[]>();
  for (const t of [...teams].sort((a, b) => a.name.localeCompare(b.name))) {
    const p = t.parentId && ids.has(t.parentId) ? t.parentId : null;
    byParent.set(p, [...(byParent.get(p) ?? []), t]);
  }
  const out: Array<{ team: TeamRow; depth: number }> = [];
  const walk = (parent: string | null, depth: number, seen: Set<string>) => {
    for (const team of byParent.get(parent) ?? []) {
      if (seen.has(team.id)) continue;
      out.push({ team, depth });
      walk(team.id, depth + 1, new Set(seen).add(team.id));
    }
  };
  walk(null, 0, new Set());
  return out;
}

function TeamRowItem({
  ws,
  team,
  depth,
  parentName,
  onMove,
  onJoin,
  joining,
}: {
  ws: string;
  team: TeamRow;
  depth: number;
  parentName: string | null;
  onMove?: () => void;
  /** Gesetzt, wenn der Aufrufer beitreten darf (`viewer.canJoin`, noch kein Mitglied). */
  onJoin?: () => void;
  joining?: boolean;
}) {
  const retired = !!team.retiredAt;
  return (
    <li className="flex items-center gap-3 px-3 py-2 text-sm">
      <span
        aria-hidden
        className="flex h-5 min-w-5 items-center justify-center rounded text-[10px] font-semibold text-white"
        style={{ backgroundColor: team.color ?? "#6b6f76" }}
      >
        {team.key.slice(0, 2)}
      </span>
      <div className="min-w-0 flex-1" style={{ paddingLeft: depth * 16 }}>
        <Link
          to={`/${ws}/settings/teams/${team.key}/general` as "/"}
          className="font-medium hover:underline"
        >
          {team.name}
        </Link>
        <p className="text-xs text-muted-foreground">
          {team.key}
          {team.icon ? ` · icon ${team.icon}` : ""}
          {parentName ? ` · in ${parentName}` : ""}
        </p>
      </div>
      <span className="w-24 text-xs text-muted-foreground capitalize">{team.visibility}</span>
      <span
        className="w-20 text-right text-xs text-muted-foreground tabular-nums"
        aria-label={`Members of ${team.name}`}
      >
        {team.memberCount == null
          ? "–"
          : `${team.memberCount} member${team.memberCount === 1 ? "" : "s"}`}
      </span>
      <span
        className={
          retired
            ? "w-16 text-right text-xs text-muted-foreground"
            : "w-16 text-right text-xs text-foreground"
        }
      >
        {retired ? "Retired" : "Active"}
      </span>
      {onJoin && (
        <Button
          size="sm"
          variant="outline"
          aria-label={`Join ${team.name}`}
          disabled={joining || retired}
          onClick={onJoin}
        >
          Join
        </Button>
      )}
      {onMove && (
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Move ${team.name}`}
          disabled={retired}
          onClick={onMove}
        >
          Move…
        </Button>
      )}
    </li>
  );
}

/** Workspace-Settings „Teams“ (`/{ws}/settings/teams`): alle sichtbaren Teams, „New team“. */
export function TeamsSettingsPage({ ws }: { ws: string }) {
  const q = useTeamsOverview();
  const session = useSession();
  const nav = workspaceSettingsNav(ws, session, "teams");
  const rows = useMemo(() => orderTeams(q.data ?? []), [q.data]);
  const names = useMemo(() => new Map((q.data ?? []).map((t) => [t.id, t.name])), [q.data]);
  const canTry = session?.user.role !== "guest";
  const admin = session?.user.role === "owner" || session?.user.role === "admin";
  const [moving, setMoving] = useState<TeamRow | null>(null);
  const join = useJoinTeam();
  const me = session?.user.id;
  return (
    <SettingsShell title="Settings" nav={nav}>
      <Section title="Teams" description="All teams you can see in this workspace.">
        {canTry && (
          <Button
            size="sm"
            className="self-start"
            onClick={() => setUiState({ createTeamOpen: true })}
          >
            <Plus /> New team
          </Button>
        )}
        {q.isLoading && <ListSkeleton rows={4} />}
        {q.isError && (
          <ErrorState
            error={q.error}
            title="Could not load teams"
            onRetry={() => void q.refetch()}
          />
        )}
        {q.data && q.data.length === 0 && <EmptyState title="No teams yet" />}
        {rows.length > 0 && (
          <ul aria-label="Teams" className="divide-y divide-border rounded-lg border border-border">
            {rows.map(({ team, depth }) => (
              <TeamRowItem
                key={team.id}
                ws={ws}
                team={team}
                depth={depth}
                parentName={team.parentId ? (names.get(team.parentId) ?? null) : null}
                onMove={admin ? () => setMoving(team) : undefined}
                onJoin={
                  me && team.viewer?.canJoin && !team.viewer.isMember
                    ? () => join.mutate({ teamId: team.id, userId: me })
                    : undefined
                }
                joining={join.isPending}
              />
            ))}
          </ul>
        )}
        {join.isError && (
          <p role="alert" className="text-xs text-destructive">
            {join.error instanceof Error ? join.error.message : "Could not join the team."}
          </p>
        )}
      </Section>
      {admin && <DeletedTeamsSection />}
      <TeamNestingDialog team={moving} all={q.data ?? []} onClose={() => setMoving(null)} />
    </SettingsShell>
  );
}
