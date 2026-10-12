import { createFileRoute } from "@tanstack/react-router";

import { AccountAppearancePage } from "@/features/settings/account-appearance";

export const Route = createFileRoute("/$ws/settings/account/appearance")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <AccountAppearancePage ws={ws} />;
}
