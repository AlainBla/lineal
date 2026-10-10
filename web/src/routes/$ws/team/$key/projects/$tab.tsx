import { createFileRoute, redirect } from "@tanstack/react-router";

import { isProjectsTab, ProjectsPage } from "@/features/planning/projects-page";

// Neue Datei außerhalb der WP14-Eigentumsliste (Team-Projektliste, nav.teamPaths().projects).
export const Route = createFileRoute("/$ws/team/$key/projects/$tab")({
  beforeLoad: ({ params }) => {
    if (!isProjectsTab(params.tab)) {
      throw redirect({ to: `/${params.ws}/team/${params.key}/projects/all` as "/" });
    }
  },
  component: Page,
});

function Page() {
  const { ws, key, tab } = Route.useParams();
  return <ProjectsPage key={key} ws={ws} teamKey={key} tab={isProjectsTab(tab) ? tab : "all"} />;
}
