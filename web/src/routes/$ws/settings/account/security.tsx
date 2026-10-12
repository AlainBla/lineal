import { createFileRoute } from "@tanstack/react-router";

import { AccountSecurityPage } from "@/features/settings/account-security";

export const Route = createFileRoute("/$ws/settings/account/security")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <AccountSecurityPage ws={ws} />;
}
