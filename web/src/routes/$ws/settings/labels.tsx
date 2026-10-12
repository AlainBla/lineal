import { createFileRoute } from "@tanstack/react-router";

import { WorkspaceLabelsPage } from "@/features/settings/workspace-config-pages";

export const Route = createFileRoute("/$ws/settings/labels")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <WorkspaceLabelsPage ws={ws} />;
}
