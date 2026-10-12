import { createFileRoute } from "@tanstack/react-router";

import { AccountNotificationsPage } from "@/features/settings/account-notifications";

export const Route = createFileRoute("/$ws/settings/account/notifications")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <AccountNotificationsPage ws={ws} />;
}
