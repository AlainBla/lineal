import { createFileRoute } from "@tanstack/react-router";

import { TriagePage } from "@/features/triage/triage-page";

export const Route = createFileRoute("/$ws/team/$key/triage")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  return <TriagePage key={key} ws={ws} teamKey={key} />;
}
