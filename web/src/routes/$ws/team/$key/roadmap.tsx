import { createFileRoute } from "@tanstack/react-router";

import { RoadmapPage } from "@/features/planning/roadmap-page";

// Neue Datei außerhalb der WP14-Eigentumsliste (Team-Roadmap).
export const Route = createFileRoute("/$ws/team/$key/roadmap")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  return <RoadmapPage key={key} ws={ws} scope={{ kind: "team", teamKey: key }} />;
}
