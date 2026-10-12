import { createFileRoute } from "@tanstack/react-router";

import { WorkspaceWebhooksPage } from "@/features/settings/team-extra-sections";

export const Route = createFileRoute("/$ws/settings/webhooks")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <WorkspaceWebhooksPage ws={ws} />;
}
