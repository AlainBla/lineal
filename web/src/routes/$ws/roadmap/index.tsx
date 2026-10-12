import { createFileRoute } from "@tanstack/react-router";

import { RoadmapPage } from "@/features/planning/roadmap-page";

export const Route = createFileRoute("/$ws/roadmap/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  return <RoadmapPage ws={ws} scope={{ kind: "workspace" }} />;
}
