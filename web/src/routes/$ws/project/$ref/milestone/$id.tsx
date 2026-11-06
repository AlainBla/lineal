import { createFileRoute } from "@tanstack/react-router";

import { MilestonePage } from "@/features/planning/milestone-page";

export const Route = createFileRoute("/$ws/project/$ref/milestone/$id")({ component: Page });

function Page() {
  const { ws, ref, id } = Route.useParams();
  return <MilestonePage key={id} ws={ws} projectRef={ref} milestoneId={id} />;
}
