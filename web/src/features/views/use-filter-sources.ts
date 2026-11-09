import { useMemo } from "react";

import { useTeamMetas, useTeams } from "@/features/issues/hooks";
import type { TeamMeta } from "@/features/issues/queries";
import { useDirectoryUsers } from "@/features/issues/users";

import type { FilterSources } from "./filter-bar";

/**
 * IDs des gewählten Teams plus aller Nachfahren (über `parentId`-Ketten). `null`, wenn kein Team
 * gewählt ist (kein Scope). Rein, damit testbar.
 */
export function teamScopeIds(
  scopeTeamId: string | undefined,
  teams: ReadonlyArray<{ id: string; parentId?: string | null }>,
): Set<string> | null {
  if (!scopeTeamId) return null;
  const set = new Set<string>([scopeTeamId]);
  for (let changed = true; changed; ) {
    changed = false;
    for (const t of teams) {
      if (t.parentId && set.has(t.parentId) && !set.has(t.id)) {
        set.add(t.id);
        changed = true;
      }
    }
  }
  return set;
}

/**
 * Wertevorräte der Filter-Bar (Personen, Labels, Projekte, Teams) aus den Team-Metadaten und `users`.
 *
 * `scopeTeamId` beschränkt die team-gebundenen Vorräte (Labels, Status, Projekte, Meilensteine, Teams)
 * auf dieses Team **und seine Unter-Teams** — passend zu einer Team-Ansicht, die Unter-Teams einschließt.
 * Ohne Scope (z. B. im team-übergreifenden View-Builder) erscheinen die Vorräte aller Teams. Personen
 * bleiben workspace-weit; die zurückgegebenen `metas`/`teams` bleiben ungescoped, damit Namens- und
 * Zustandsauflösung auch für Tickets aus anderen Teams funktioniert.
 */
export function useFilterSources(scopeTeamId?: string): {
  sources: FilterSources;
  metas: Array<TeamMeta | undefined>;
  teams: ReturnType<typeof useTeams>;
} {
  const teams = useTeams();
  const metas = useTeamMetas((teams.data ?? []).map((t) => t.key));
  const users = useDirectoryUsers();
  // Erlaubte Team-IDs: das gewählte Team plus alle Nachfahren (über `parentId`-Ketten). Null = kein Scope.
  const allowed = useMemo(
    () => teamScopeIds(scopeTeamId, teams.data ?? []),
    [scopeTeamId, teams.data],
  );
  const sources = useMemo<FilterSources>(() => {
    const scoped = allowed ? metas.filter((m) => m && allowed.has(m.team.id)) : metas;
    return {
      users: (users.data ?? []).map((u) => ({ id: u.id, name: u.displayName || u.name })),
      labels: scoped.flatMap((m) => (m ? m.labels.filter((l) => !l.isGroup) : [])),
      projects: scoped.flatMap((m) => m?.projects ?? []),
      teams: (teams.data ?? [])
        .filter((t) => !allowed || allowed.has(t.id))
        .map((t) => ({ id: t.id, name: t.name })),
      states: scoped.flatMap((m) =>
        m ? m.states.map((s) => ({ id: s.id, name: s.name, team: m.team.key })) : [],
      ),
      milestones: scoped
        .flatMap((m) => m?.projects ?? [])
        .flatMap((p) => (p.milestones ?? []).map((ms) => ({ id: ms.id, name: ms.name }))),
    };
  }, [users.data, metas, teams.data, allowed]);
  return { sources, metas, teams };
}
