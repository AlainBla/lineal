import { Link } from "@tanstack/react-router";
import { useMemo } from "react";

import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import type { Project } from "@/gql/graphql";

import { useProjects } from "./queries";
import { DateText, HealthBadge, PageHeader, ProgressBar } from "./ui";

export type ProjectsTab = "all" | "active" | "backlog";
export const isProjectsTab = (v: string): v is ProjectsTab =>
  v === "all" || v === "active" || v === "backlog";

/** Filter der Tabs: active = laufend, backlog = geplant/Backlog (docs/05 §3.3). */
export function filterProjects(projects: Project[], tab: ProjectsTab): Project[] {
  if (tab === "all") return projects;
  if (tab === "active") return projects.filter((p) => p.status.type === "started");
  return projects.filter((p) => p.status.type === "backlog" || p.status.type === "planned");
}

/** Projektliste (workspace- oder teamweit, docs/08 §2.7 / 05 §3.3). */
export function ProjectsPage({
  ws,
  tab,
  teamKey,
}: {
  ws: string;
  tab: ProjectsTab;
  teamKey?: string;
}) {
  const q = useProjects({ team: teamKey, includeSubTeams: teamKey ? true : undefined });
  const rows = useMemo(() => filterProjects(q.data ?? [], tab), [q.data, tab]);
  const base = teamKey ? `/${ws}/team/${teamKey}/projects` : `/${ws}/projects`;

  let body;
  if (q.isLoading) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState
        error={q.error}
        title="Could not load projects"
        onRetry={() => void q.refetch()}
      />
    );
  else if (rows.length === 0)
    body = <EmptyState title="No projects" description="Projects you can access show up here." />;
  else
    body = (
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs text-muted-foreground">
            <th className="px-4 py-2 font-medium">Name</th>
            <th className="px-3 py-2 font-medium">Health</th>
            <th className="px-3 py-2 font-medium">Lead</th>
            <th className="px-3 py-2 font-medium">Teams</th>
            <th className="px-3 py-2 font-medium">Target</th>
            <th className="px-3 py-2 font-medium">Progress</th>
            <th className="px-3 py-2 font-medium">Status</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={p.id} className="border-b border-border hover:bg-surface-hover">
              <td className="px-4 py-1.5">
                <Link
                  to={`/${ws}/project/${p.identifier}/overview` as "/"}
                  className="flex items-center gap-2 hover:underline"
                >
                  <span
                    aria-hidden
                    className="size-2.5 rounded-sm"
                    style={{ background: /^#[0-9A-Fa-f]{6}$/.test(p.color) ? p.color : undefined }}
                  />
                  {p.name}
                </Link>
              </td>
              <td className="px-3 py-1.5">
                <HealthBadge health={p.health} />
              </td>
              <td className="px-3 py-1.5 text-muted-foreground">{p.lead?.name ?? "–"}</td>
              <td className="px-3 py-1.5 text-muted-foreground">
                {p.teams.map((t) => t.key ?? t.name).join(", ")}
              </td>
              <td className="px-3 py-1.5 text-muted-foreground">
                <DateText iso={p.targetDate} resolution={p.targetDateResolution} />
              </td>
              <td className="px-3 py-1.5">
                <ProgressBar value={p.progress} label={`Progress of ${p.name}`} />
              </td>
              <td className="px-3 py-1.5 text-muted-foreground">{p.status.name}</td>
            </tr>
          ))}
        </tbody>
      </table>
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Projects"
        docTitle={teamKey ? `${teamKey} · Projects` : "Projects"}
        tabs={[
          ...(["all", "active", "backlog"] as const).map((t) => ({
            label: t === "all" ? "All projects" : t === "active" ? "Active" : "Backlog",
            to: `${base}/${t}`,
            active: t === tab,
          })),
          {
            label: "Roadmap",
            to: teamKey ? `/${ws}/team/${teamKey}/roadmap` : `/${ws}/roadmap`,
            active: false,
          },
        ]}
      />
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>
    </div>
  );
}
