import { createFileRoute, Outlet, redirect } from "@tanstack/react-router";

import { ensureSession } from "@/app/auth";

/**
 * Layout aller Workspace-Screens (`/{ws}/…`, docs/08 §1.3). Gemeinsame Datei (Orchestrator):
 * Feature-Routen liegen unter `routes/$ws/<bereich>/…` und gehören je einem Arbeitspaket.
 * Ein fremder Workspace-Key leitet auf denselben Pfad im eigenen Workspace um.
 */
export const Route = createFileRoute("/$ws")({
  beforeLoad: async ({ params, location }) => {
    const session = await ensureSession();
    if (session && params.ws !== session.workspace.urlKey) {
      const rest = location.pathname.slice(params.ws.length + 1);
      throw redirect({ to: `/${session.workspace.urlKey}${rest}` as "/" });
    }
  },
  component: Outlet,
});
