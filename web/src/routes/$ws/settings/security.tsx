import { createFileRoute } from "@tanstack/react-router";

import { SecuritySettingsPage } from "@/features/settings/security-page";

export const Route = createFileRoute("/$ws/settings/security")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <SecuritySettingsPage ws={ws} />;
}
