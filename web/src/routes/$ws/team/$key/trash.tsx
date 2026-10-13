import { createFileRoute } from "@tanstack/react-router";

import { TrashPage } from "@/features/trash/trash-page";

export const Route = createFileRoute("/$ws/team/$key/trash")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  return <TrashPage key={key} ws={ws} teamKey={key} />;
}
