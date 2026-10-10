import { createFileRoute } from "@tanstack/react-router";

import { DocumentPage } from "@/features/docs/document-page";

export const Route = createFileRoute("/$ws/document/$ref")({ component: Page });

function Page() {
  const { ref } = Route.useParams();
  return <DocumentPage key={ref} docRef={ref} />;
}
