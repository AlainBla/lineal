import { createFileRoute, redirect } from "@tanstack/react-router";

import { isProjectsTab, ProjectsPage } from "@/features/planning/projects-page";

export const Route = createFileRoute("/$ws/projects/$tab")({
  beforeLoad: ({ params }) => {
    if (!isProjectsTab(params.tab)) throw redirect({ to: `/${params.ws}/projects/all` as "/" });
  },
  component: Page,
});

function Page() {
  const { ws, tab } = Route.useParams();
  return <ProjectsPage ws={ws} tab={isProjectsTab(tab) ? tab : "all"} />;
}
