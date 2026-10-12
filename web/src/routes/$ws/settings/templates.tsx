import { createFileRoute } from "@tanstack/react-router";

import { WorkspaceTemplatesPage } from "@/features/settings/workspace-config-pages";

export const Route = createFileRoute("/$ws/settings/templates")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <WorkspaceTemplatesPage ws={ws} />;
}
