import { createFileRoute } from "@tanstack/react-router";

import { AuditLogPage } from "@/features/settings/audit-page";

export const Route = createFileRoute("/$ws/settings/audit")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <AuditLogPage ws={ws} />;
}
