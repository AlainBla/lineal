import { createFileRoute, redirect } from "@tanstack/react-router";

import { InitiativeDetail, isInitiativeTab } from "@/features/planning/initiative-detail";

// Tabs: overview | projects | updates | documents | sub-initiatives | roadmap.
export const Route = createFileRoute("/$ws/initiative/$ref/$tab")({
  beforeLoad: ({ params, location }) => {
    if (!isInitiativeTab(params.tab)) {
      throw redirect({
        to: `/${params.ws}/initiative/${params.ref}/overview` as "/",
        hash: location.hash,
      });
    }
  },
  component: Page,
});

function Page() {
  const { ws, ref, tab } = Route.useParams();
  return (
    <InitiativeDetail
      key={ref}
      ws={ws}
      initiativeRef={ref}
      tab={isInitiativeTab(tab) ? tab : "overview"}
    />
  );
}
