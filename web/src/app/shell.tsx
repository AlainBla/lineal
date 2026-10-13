import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, type ReactNode } from "react";

import { logout, type Session } from "@/app/auth";
import { ConnectedCommandMenu } from "@/app/command-menu";
import { useGlobalActions } from "@/app/global-actions";
import { ShortcutHelp } from "@/app/shortcut-help";
import { Sidebar } from "@/app/sidebar";
import { Topbar } from "@/app/topbar";
import { clearAllCreateDrafts } from "@/features/issues/create-store";
import { IssuesHost } from "@/features/issues/host";
import { ConnectedNewTeamDialog } from "@/features/settings/new-team-dialog";
import { attachShortcuts } from "@/lib/shortcuts";
import { useSyncStream } from "@/lib/store";

/** Linear-artige Shell: Sidebar, Topbar, Hauptbereich, Overlays und globale Tastaturbedienung. */
export function Shell({ session, children }: { session: Session; children: ReactNode }) {
  const navigate = useNavigate();
  const client = useQueryClient();

  const onLogout = () => {
    void logout().then(() => {
      clearAllCreateDrafts();
      client.clear();
      void navigate({ to: "/login", search: { redirect: undefined } });
    });
  };

  useGlobalActions({ ws: session.workspace.urlKey, onLogout });
  useEffect(() => attachShortcuts(), []);
  useSyncStream();

  return (
    <div className="flex h-dvh w-full overflow-hidden bg-background text-foreground">
      <a
        href="#main"
        className="sr-only z-50 rounded-md bg-popover px-3 py-1.5 shadow-popover focus:not-sr-only focus:absolute focus:top-2 focus:left-2"
      >
        Skip to content
      </a>
      <Sidebar session={session} onLogout={onLogout} />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar />
        <main id="main" tabIndex={-1} className="min-h-0 flex-1 overflow-y-auto outline-none">
          {children}
        </main>
      </div>
      <ConnectedCommandMenu />
      <ShortcutHelp />
      <IssuesHost />
      <ConnectedNewTeamDialog />
    </div>
  );
}
