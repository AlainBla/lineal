/** Nachschlagetabellen (Status, Labels, Projekte, Personen) für Zeilen, Karten und Detail. */
import { useMemo } from "react";

import { useSession } from "@/app/auth";
import type { IssueLabel, WorkflowState } from "@/gql/graphql";

import type { TeamMeta } from "./queries";
import { useKnownUsers, userName } from "./users";

export interface IssueLookup {
  state: (id: string) => WorkflowState | undefined;
  label: (id: string) => IssueLabel | undefined;
  projectName: (id: string | null) => string | null;
  assigneeName: (id: string | null) => string | null;
  /** Schätzskala je Team (`notUsed`, wenn unbekannt). */
  estimationType: (teamId: string) => string;
}

export function useIssueLookup(metas: Array<TeamMeta | undefined>): IssueLookup {
  const users = useKnownUsers();
  const session = useSession();
  const me = session ? { id: session.user.id, name: session.user.name } : null;
  return useMemo(() => {
    const all = metas.filter((m): m is TeamMeta => !!m);
    const states = new Map(all.flatMap((m) => m.states.map((s) => [s.id, s] as const)));
    const labels = new Map(all.flatMap((m) => m.labels.map((l) => [l.id, l] as const)));
    const projects = new Map(all.flatMap((m) => m.projects.map((p) => [p.id, p.name] as const)));
    const estimation = new Map(all.map((m) => [m.team.id, m.team.issueEstimationType] as const));
    return {
      state: (id) => states.get(id),
      label: (id) => labels.get(id),
      projectName: (id) => (id ? (projects.get(id) ?? null) : null),
      assigneeName: (id) => userName(id, users, me),
      estimationType: (teamId) => estimation.get(teamId) ?? "notUsed",
    };
    // `me` ist pro Session stabil genug; Objektidentität würde jede Renderrunde neu rechnen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [metas, users, me?.id, me?.name]);
}
