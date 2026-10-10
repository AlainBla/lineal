import { createFileRoute } from "@tanstack/react-router";

import { InboxPage } from "@/features/inbox/inbox-page";

export const Route = createFileRoute("/$ws/inbox/")({
  component: Page,
});

function Page() {
  const { ws } = Route.useParams();
  return <InboxPage ws={ws} />;
}
