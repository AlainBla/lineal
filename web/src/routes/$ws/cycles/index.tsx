import { createFileRoute } from "@tanstack/react-router";

import { CyclesOverviewPage } from "@/features/overview/cycles-overview-page";

export const Route = createFileRoute("/$ws/cycles/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <CyclesOverviewPage ws={ws} />;
}
