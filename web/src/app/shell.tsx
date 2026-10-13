import { useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";

import { logout, type Session } from "@/app/auth";
import { ConnectedCommandMenu } from "@/app/command-menu";
import { useGlobalActions } from "@/app/global-actions";
import { titleForPath } from "@/app/nav";
import { ShortcutHelp } from "@/app/shortcut-help";
import { Sidebar } from "@/app/sidebar";
import { Topbar } from "@/app/topbar";
import { setUiState, useUiState } from "@/app/ui-state";
import { clearAllCreateDrafts } from "@/features/issues/create-store";
import { clearUndo } from "@/features/issues/undo";
import { RemindHost } from "@/features/inbox/remind";
import { IssuesHost } from "@/features/issues/host";
import { ConnectedNewTeamDialog } from "@/features/settings/new-team-dialog";
import { attachShortcuts } from "@/lib/shortcuts";
import { NARROW_QUERY, useMediaQuery } from "@/lib/use-media-query";
import { useSyncStream } from "@/lib/store";

const pageTitle = (pathname: string) => `${titleForPath(pathname)} · Lineal`;

/** Linear-artige Shell: Sidebar, Topbar, Hauptbereich, Overlays und globale Tastaturbedienung. */
export function Shell({ session, children }: { session: Session; children: ReactNode }) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const router = useRouter();

  // Standardtitel je Pfad (B4.32): beim ersten Render (vor den Effekten der Seite) und bei jeder
  // Navigation zurückgesetzt, damit nie der Titel der vorherigen Seite stehen bleibt. Seiten mit
  // eigenem Titel überschreiben ihn danach in ihrem Effekt.
  useState(() => {
    document.title = pageTitle(router.state.location.pathname);
  });
  useEffect(() => {
    const offBefore = router.subscribe("onBeforeNavigate", ({ toLocation }) => {
      document.title = pageTitle(toLocation.pathname);
    });
    // Seiten, die ihren Titel beim Verlassen auf den vorherigen Wert zurücksetzen, würden den neuen
    // Standardtitel überschreiben: steht nach dem Rendern noch der Standardtitel der alten Seite,
    // gilt wieder der der neuen.
    const offRendered = router.subscribe("onRendered", ({ fromLocation, toLocation }) => {
      if (fromLocation && fromLocation.pathname !== toLocation.pathname) {
        if (document.title === pageTitle(fromLocation.pathname))
          document.title = pageTitle(toLocation.pathname);
      }
    });
    return () => {
      offBefore();
      offRendered();
    };
  }, [router]);

  const onLogout = () => {
    void logout().then(() => {
      clearAllCreateDrafts();
      clearUndo();
      client.clear();
      void navigate({ to: "/login", search: { redirect: undefined } });
    });
  };

  const narrow = useMediaQuery(NARROW_QUERY);
  const { sidebarOpen } = useUiState();
  const pathname = useRouterState({ select: (st) => st.location.pathname });
  // Schmal: Sidebar ist ein Overlay; sie klappt beim Start und nach jeder Navigation ein (B4.28),
  // ohne die gespeicherte Desktop-Einstellung zu ändern.
  useEffect(() => {
    if (narrow) setUiState({ sidebarOpen: false }, { persist: false });
  }, [narrow, pathname]);

  useGlobalActions({
    ws: session.workspace.urlKey,
    onLogout,
    guest: session.user.role === "guest",
  });
  useEffect(() => attachShortcuts(), []);
  useSyncStream();

  return (
    <div className="flex h-dvh w-full overflow-hidden bg-background text-foreground">
      <a
        href="#main"
        // Fokus statt Hash-Navigation: kein zusätzlicher History-Eintrag (B4.29).
        onClick={(e) => {
          e.preventDefault();
          document.getElementById("main")?.focus();
        }}
        className="sr-only z-50 rounded-md bg-popover px-3 py-1.5 shadow-popover focus:not-sr-only focus:absolute focus:top-2 focus:left-2"
      >
        Skip to content
      </a>
      {narrow && sidebarOpen && (
        <div
          aria-hidden
          data-testid="sidebar-scrim"
          className="fixed inset-0 z-30 bg-scrim"
          onClick={() => setUiState({ sidebarOpen: false }, { persist: false })}
        />
      )}
      <Sidebar session={session} onLogout={onLogout} overlay={narrow} />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar />
        <main id="main" tabIndex={-1} className="min-h-0 flex-1 overflow-y-auto outline-none">
          {children}
        </main>
      </div>
      <ConnectedCommandMenu />
      <ShortcutHelp />
      <IssuesHost />
      <RemindHost />
      <ConnectedNewTeamDialog />
    </div>
  );
}
