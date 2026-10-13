import { createFileRoute, redirect } from "@tanstack/react-router";

import { isTrashTab, TrashPage } from "@/features/trash/trash-page";

export const Route = createFileRoute("/$ws/trash/$tab")({
  beforeLoad: ({ params }) => {
    if (!isTrashTab(params.tab)) throw redirect({ to: `/${params.ws}/trash/issues` as "/" });
  },
  component: Page,
});

function Page() {
  const { ws, tab } = Route.useParams();
  return <TrashPage ws={ws} tab={isTrashTab(tab) ? tab : "issues"} />;
}
