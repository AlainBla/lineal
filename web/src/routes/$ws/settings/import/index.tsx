import { createFileRoute } from "@tanstack/react-router";

import { ImportPage } from "@/features/import/import-page";

export const Route = createFileRoute("/$ws/settings/import/")({
  component: ImportPage,
});
