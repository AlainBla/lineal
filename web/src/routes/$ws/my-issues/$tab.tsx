import { createFileRoute } from "@tanstack/react-router";

import { MyIssuesPage } from "@/features/issues/my-issues-page";

export const Route = createFileRoute("/$ws/my-issues/$tab")({
  component: Page,
});

function Page() {
  const { ws, tab } = Route.useParams();
  return <MyIssuesPage ws={ws} tab={tab} />;
}
