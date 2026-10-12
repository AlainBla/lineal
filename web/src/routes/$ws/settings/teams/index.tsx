import { createFileRoute } from "@tanstack/react-router";

import { TeamsSettingsPage } from "@/features/settings/teams-page";

export const Route = createFileRoute("/$ws/settings/teams/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <TeamsSettingsPage ws={ws} />;
}
