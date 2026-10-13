import { createFileRoute } from "@tanstack/react-router";

import { TeamDocumentsPage } from "@/features/docs/team-documents-page";

export const Route = createFileRoute("/$ws/team/$key/documents")({ component: Page });

function Page() {
  const { ws, key } = Route.useParams();
  return <TeamDocumentsPage key={key} ws={ws} teamKey={key} />;
}
