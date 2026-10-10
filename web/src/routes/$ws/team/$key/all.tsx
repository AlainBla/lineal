import { createFileRoute } from "@tanstack/react-router";

import { TeamIssuesPage } from "@/features/issues/team-issues-page";

export const Route = createFileRoute("/$ws/team/$key/all")({
  component: Page,
});

function Page() {
  const { ws, key } = Route.useParams();
  return <TeamIssuesPage key={`${key}/all`} ws={ws} teamKey={key} view="all" />;
}
