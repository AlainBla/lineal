import { createFileRoute } from "@tanstack/react-router";

import { IssueDetailPage } from "@/features/issues/issue-detail";

export const Route = createFileRoute("/$ws/issue/$")({
  component: Page,
});

function Page() {
  const { ws, _splat } = Route.useParams();
  return <IssueDetailPage ws={ws} splat={_splat ?? ""} />;
}
