import { createFileRoute, redirect } from "@tanstack/react-router";

import { isTeamSection, TeamSettingsPage } from "@/features/settings/settings-pages";

export const Route = createFileRoute("/$ws/settings/teams/$key/$section")({
  beforeLoad: ({ params }) => {
    if (!isTeamSection(params.section)) {
      throw redirect({ to: `/${params.ws}/settings/teams/${params.key}/general` as "/" });
    }
  },
  component: Page,
});

function Page() {
  const { ws, key, section } = Route.useParams();
  return (
    <TeamSettingsPage
      key={key}
      ws={ws}
      teamKey={key}
      section={isTeamSection(section) ? section : "general"}
    />
  );
}
