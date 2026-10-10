import { createFileRoute } from "@tanstack/react-router";

import { CyclePage } from "@/features/planning/cycles-page";

// Neue Datei außerhalb der WP14-Eigentumsliste (Cycle-Detail, nav.teamPaths().currentCycle/upcomingCycle).
export const Route = createFileRoute("/$ws/team/$key/cycle/$ref")({ component: Page });

function Page() {
  const { ws, key, ref } = Route.useParams();
  return <CyclePage key={`${key}/${ref}`} ws={ws} teamKey={key} cycleRef={ref} />;
}
