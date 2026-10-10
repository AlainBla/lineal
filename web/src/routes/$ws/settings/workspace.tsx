import { createFileRoute } from "@tanstack/react-router";

import { WorkspaceSettingsPage } from "@/features/settings/settings-pages";

export const Route = createFileRoute("/$ws/settings/workspace")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <WorkspaceSettingsPage ws={ws} />;
}
