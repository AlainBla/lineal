import { createFileRoute, redirect } from "@tanstack/react-router";

import { InitiativesPage, isInitiativesTab } from "@/features/planning/initiatives-page";

export const Route = createFileRoute("/$ws/initiatives/$tab")({
  beforeLoad: ({ params }) => {
    if (!isInitiativesTab(params.tab)) {
      throw redirect({ to: `/${params.ws}/initiatives/active` as "/" });
    }
  },
  component: Page,
});

function Page() {
  const { ws, tab } = Route.useParams();
  return <InitiativesPage ws={ws} tab={isInitiativesTab(tab) ? tab : "active"} />;
}
