import { createFileRoute, redirect } from "@tanstack/react-router";

import { isProjectTab, ProjectPage } from "@/features/planning/project-page";

export const Route = createFileRoute("/$ws/project/$ref/$tab")({
  beforeLoad: ({ params }) => {
    if (!isProjectTab(params.tab)) {
      throw redirect({ to: `/${params.ws}/project/${params.ref}/overview` as "/" });
    }
  },
  component: Page,
});

function Page() {
  const { ws, ref, tab } = Route.useParams();
  return (
    <ProjectPage key={ref} ws={ws} projectRef={ref} tab={isProjectTab(tab) ? tab : "overview"} />
  );
}
