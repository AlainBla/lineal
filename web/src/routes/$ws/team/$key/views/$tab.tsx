import { createFileRoute } from "@tanstack/react-router";

import { useSession } from "@/app/auth";
import { ViewsPage } from "@/features/views/views-page";

// Neue Datei außerhalb der WP14-Eigentumsliste (Team-Views, nav.teamPaths().views).
export const Route = createFileRoute("/$ws/team/$key/views/$tab")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  const session = useSession();
  const team = session?.teams.find((t) => t.key === key);
  return <ViewsPage key={key} ws={ws} teamKey={key} teamId={team?.id} />;
}
