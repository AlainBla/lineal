import { useEffect, useMemo } from "react";

import { useTeamMetas, useTeams } from "@/features/issues/hooks";
import type { TeamMeta } from "@/features/issues/queries";
import { rememberUsers } from "@/features/issues/users";

import type { FilterSources } from "./filter-bar";
import { useUsers } from "./queries";

/** Wertevorräte der Filter-Bar (Personen, Labels, Projekte, Teams) aus den Team-Metadaten und `users`. */
export function useFilterSources(): {
  sources: FilterSources;
  metas: Array<TeamMeta | undefined>;
  teams: ReturnType<typeof useTeams>;
} {
  const teams = useTeams();
  const metas = useTeamMetas((teams.data ?? []).map((t) => t.key));
  const users = useUsers();
  useEffect(() => {
    if (users.data) {
      rememberUsers(users.data.map((u) => ({ id: u.id, name: u.displayName || u.name })));
    }
  }, [users.data]);
  const sources = useMemo<FilterSources>(
    () => ({
      users: (users.data ?? []).map((u) => ({ id: u.id, name: u.displayName || u.name })),
      labels: metas.flatMap((m) => (m ? m.labels.filter((l) => !l.isGroup) : [])),
      projects: metas.flatMap((m) => m?.projects ?? []),
      teams: (teams.data ?? []).map((t) => ({ id: t.id, name: t.name })),
    }),
    [users.data, metas, teams.data],
  );
  return { sources, metas, teams };
}
