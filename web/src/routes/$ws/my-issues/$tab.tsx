import { createFileRoute, redirect } from "@tanstack/react-router";

import { MyIssuesPage } from "@/features/issues/my-issues-page";

const TABS = ["assigned", "created", "subscribed"];

export const Route = createFileRoute("/$ws/my-issues/$tab")({
  // Ungültige Tabs leiten still auf Assigned um (docs/08 §1.4).
  beforeLoad: ({ params }) => {
    if (!TABS.includes(params.tab)) {
      throw redirect({ to: `/${params.ws}/my-issues/assigned` as "/" });
    }
  },
  component: Page,
});

function Page() {
  const { ws, tab } = Route.useParams();
  return <MyIssuesPage ws={ws} tab={tab} />;
}
