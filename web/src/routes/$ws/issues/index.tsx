import { createFileRoute } from "@tanstack/react-router";

import { WorkspaceIssuesPage } from "@/features/overview/workspace-issues-page";

export const Route = createFileRoute("/$ws/issues/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <WorkspaceIssuesPage ws={ws} />;
}
