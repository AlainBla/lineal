/** Workspace-Settings „Labels“ und „Templates“ (WP27): `/{ws}/settings/labels` und `/templates`. */
import { useSession } from "@/app/auth";
import { ErrorState, ListSkeleton } from "@/features/issues/states";

import { LabelManager } from "./label-manager";
import { workspaceSettingsNav } from "./nav";
import { useWorkspaceLabels } from "./template-queries";
import { TemplatesManager } from "./templates-manager";
import { SettingsShell } from "./ui";

export function WorkspaceLabelsPage({ ws }: { ws: string }) {
  const session = useSession();
  const q = useWorkspaceLabels();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "labels")}>
      {q.isLoading && <ListSkeleton rows={4} />}
      {q.isError && (
        <ErrorState
          error={q.error}
          title="Could not load labels"
          onRetry={() => void q.refetch()}
        />
      )}
      {q.data && (
        <LabelManager
          title="Labels"
          description="Workspace labels are available in all teams."
          labels={q.data}
        />
      )}
    </SettingsShell>
  );
}

export function WorkspaceTemplatesPage({ ws }: { ws: string }) {
  const session = useSession();
  return (
    <SettingsShell title="Settings" nav={workspaceSettingsNav(ws, session, "templates")}>
      <TemplatesManager />
    </SettingsShell>
  );
}
