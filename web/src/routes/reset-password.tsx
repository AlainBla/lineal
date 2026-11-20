import { createFileRoute } from "@tanstack/react-router";
import { useEffect } from "react";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { queryClient } from "@/app/query-client";
import { ResetPasswordPage } from "@/features/password-reset/reset-password-page";

/** Öffentlich: der Auth-Guard im Root lässt `/reset-password` ohne Session durch. */
export const Route = createFileRoute("/reset-password")({
  validateSearch: (search: Record<string, unknown>): { token?: string } => ({
    token: typeof search.token === "string" ? search.token : undefined,
  }),
  component: Page,
});

function Page() {
  const { token } = Route.useSearch();

  useEffect(() => {
    // Der Token steht in der URL: nicht per Referer an Dritte weitergeben.
    const meta = document.createElement("meta");
    meta.name = "referrer";
    meta.content = "no-referrer";
    document.head.appendChild(meta);
    return () => meta.remove();
  }, []);

  return (
    <ResetPasswordPage
      token={token ?? ""}
      onDone={() => {
        // Kein Auto-Login (LIN-58): etwaigen Session-Cache verwerfen; die Bestätigungskarte führt
        // den Nutzer per Link frisch zum Login.
        queryClient.removeQueries({ queryKey: SESSION_QUERY_KEY });
      }}
    />
  );
}
