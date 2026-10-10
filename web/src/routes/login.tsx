import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router";

import { ensureSession, safeRedirect, SESSION_QUERY_KEY } from "@/app/auth";
import { LoginForm } from "@/app/login-form";
import { queryClient } from "@/app/query-client";

export const Route = createFileRoute("/login")({
  validateSearch: (search: Record<string, unknown>): { redirect?: string } => ({
    redirect: typeof search.redirect === "string" ? search.redirect : undefined,
  }),
  beforeLoad: async ({ search }) => {
    if (await ensureSession()) throw redirect({ to: safeRedirect(search.redirect) as "/" });
  },
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
  const { redirect: target } = Route.useSearch();

  const onSuccess = async () => {
    queryClient.removeQueries({ queryKey: SESSION_QUERY_KEY });
    await navigate({ to: safeRedirect(target) as "/" });
  };

  return (
    <main className="flex min-h-dvh items-center justify-center bg-background p-4">
      <div className="w-full max-w-sm rounded-xl border border-border-strong bg-card p-6 shadow-popover">
        <div className="mb-6 flex flex-col gap-1">
          <div
            aria-hidden
            className="mb-3 flex size-8 items-center justify-center rounded-lg bg-primary text-sm font-bold text-primary-foreground"
          >
            L
          </div>
          <h1 className="text-lg font-semibold tracking-tight">Log in to Lineal</h1>
          <p className="text-sm text-muted-foreground">Use your workspace email and password.</p>
        </div>
        <LoginForm onSuccess={() => void onSuccess()} />
      </div>
    </main>
  );
}
