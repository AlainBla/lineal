import { createFileRoute } from "@tanstack/react-router";

import { ViewsPage } from "@/features/views/views-page";

export const Route = createFileRoute("/$ws/views/$tab")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <ViewsPage ws={ws} />;
}
