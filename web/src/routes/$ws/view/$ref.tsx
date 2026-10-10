import { createFileRoute } from "@tanstack/react-router";

import { ViewPage } from "@/features/views/views-page";

export const Route = createFileRoute("/$ws/view/$ref")({ component: Page });

function Page() {
  const { ws, ref } = Route.useParams();
  return <ViewPage key={ref} ws={ws} viewRef={ref} />;
}
