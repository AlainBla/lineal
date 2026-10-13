import { createFileRoute } from "@tanstack/react-router";

import { DraftsPage } from "@/features/issues/drafts-page";

export const Route = createFileRoute("/$ws/drafts/")({
  component: Page,
});

function Page() {
  const { ws } = Route.useParams();
  return <DraftsPage ws={ws} />;
}
