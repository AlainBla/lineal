import { createFileRoute } from "@tanstack/react-router";

import { InitiativePage } from "@/features/planning/initiatives-page";

// Tabs (overview|projects|updates) zeigen vorerst dieselbe Seite.
export const Route = createFileRoute("/$ws/initiative/$ref/$tab")({ component: Page });

function Page() {
  const { ws, ref } = Route.useParams();
  return <InitiativePage key={ref} ws={ws} initiativeRef={ref} />;
}
