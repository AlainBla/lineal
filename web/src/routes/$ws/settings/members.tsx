import { createFileRoute } from "@tanstack/react-router";

import { MembersSettingsPage } from "@/features/settings/members-page";

export const Route = createFileRoute("/$ws/settings/members")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <MembersSettingsPage ws={ws} />;
}
