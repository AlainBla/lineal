import { createFileRoute } from "@tanstack/react-router";

import { useSession } from "@/app/auth";
import { ImportPage } from "@/features/import/import-page";
import { workspaceSettingsNav } from "@/features/settings/nav";
import { SettingsShell } from "@/features/settings/ui";

export const Route = createFileRoute("/$ws/settings/import/")({ component: Page });

function Page() {
  const { ws } = Route.useParams();
  const session = useSession();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "import")}>
      <ImportPage />
    </SettingsShell>
  );
}
