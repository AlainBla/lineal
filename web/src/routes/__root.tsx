import { createRootRoute, Outlet, redirect, useRouter } from "@tanstack/react-router";

import { ensureSession, useSession } from "@/app/auth";
import { PlaceholderPage } from "@/app/placeholder";
import { Shell } from "@/app/shell";
import { Button } from "@/components/ui/button";

export const Route = createRootRoute({
  // Auth-Guard: ohne gültige Session (GET /auth/me) geht jede Seite außer /login auf /login.
  beforeLoad: async ({ location }) => {
    const session = await ensureSession();
    if (!session && location.pathname !== "/login") {
      throw redirect({ to: "/login", search: { redirect: location.href } });
    }
  },
  component: RootLayout,
  // Alle Pfade, die ein späteres Arbeitspaket baut, zeigen bis dahin eine Platzhalterseite in der Shell.
  notFoundComponent: () => <PlaceholderPage />,
  errorComponent: RootError,
});

function RootLayout() {
  const session = useSession();
  if (!session) return <Outlet />;
  return (
    <Shell session={session}>
      <Outlet />
    </Shell>
  );
}

function RootError({ error }: { error: unknown }) {
  const router = useRouter();
  return (
    <main className="flex min-h-dvh flex-col items-center justify-center gap-3 p-8 text-center">
      <h1 className="text-base font-semibold">Something went wrong</h1>
      <p className="max-w-sm text-sm text-muted-foreground">
        {error instanceof Error ? error.message : "Unexpected error."}
      </p>
      <Button variant="outline" onClick={() => void router.invalidate()}>
        Try again
      </Button>
    </main>
  );
}
