import { useMemo } from "react";

import { useTeamMetas, useTeams } from "@/features/issues/hooks";
import type { TeamMeta } from "@/features/issues/queries";
import { useDirectoryUsers } from "@/features/issues/users";

import type { FilterSources } from "./filter-bar";

/** Wertevorräte der Filter-Bar (Personen, Labels, Projekte, Teams) aus den Team-Metadaten und `users`. */
export function useFilterSources(): {
  sources: FilterSources;
  metas: Array<TeamMeta | undefined>;
  teams: ReturnType<typeof useTeams>;
} {
  const teams = useTeams();
  const metas = useTeamMetas((teams.data ?? []).map((t) => t.key));
  const users = useDirectoryUsers();
  const sources = useMemo<FilterSources>(
    () => ({
      users: (users.data ?? []).map((u) => ({ id: u.id, name: u.displayName || u.name })),
      labels: metas.flatMap((m) => (m ? m.labels.filter((l) => !l.isGroup) : [])),
      projects: metas.flatMap((m) => m?.projects ?? []),
      teams: (teams.data ?? []).map((t) => ({ id: t.id, name: t.name })),
      states: metas.flatMap((m) =>
        m ? m.states.map((s) => ({ id: s.id, name: s.name, team: m.team.key })) : [],
      ),
      milestones: metas
        .flatMap((m) => m?.projects ?? [])
        .flatMap((p) => (p.milestones ?? []).map((ms) => ({ id: ms.id, name: ms.name }))),
    }),
    [users.data, metas, teams.data],
  );
  return { sources, metas, teams };
}
