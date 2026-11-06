import { createFileRoute } from "@tanstack/react-router";

import { MilestonesPage } from "@/features/planning/milestones-page";

export const Route = createFileRoute("/$ws/milestones/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <MilestonesPage ws={ws} />;
}
