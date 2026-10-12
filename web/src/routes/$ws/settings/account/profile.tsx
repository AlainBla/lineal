import { createFileRoute } from "@tanstack/react-router";

import { AccountProfilePage } from "@/features/settings/account-profile";

export const Route = createFileRoute("/$ws/settings/account/profile")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <AccountProfilePage ws={ws} />;
}
