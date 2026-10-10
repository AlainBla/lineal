import { createFileRoute } from "@tanstack/react-router";

import { OverviewPage } from "@/features/overview/overview-page";

export const Route = createFileRoute("/$ws/overview/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <OverviewPage ws={ws} />;
}
