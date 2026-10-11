import { createFileRoute } from "@tanstack/react-router";

import { ApiSettingsPage } from "@/features/settings/api-keys";

export const Route = createFileRoute("/$ws/settings/api")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <ApiSettingsPage ws={ws} />;
}
