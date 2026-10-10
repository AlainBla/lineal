import { createFileRoute } from "@tanstack/react-router";

import { TeamCyclesPage } from "@/features/planning/cycles-page";

// Neue Datei außerhalb der WP14-Eigentumsliste (Team-Cycles, nav.teamPaths().cycles).
export const Route = createFileRoute("/$ws/team/$key/cycles")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  return <TeamCyclesPage key={key} ws={ws} teamKey={key} />;
}
