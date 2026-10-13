/**
 * Abgelaufene Session (HTTP 401 auf GraphQL): zurück zum Login mit Rücksprungadresse, statt die
 * Seite durch eine Fehlerkarte zu ersetzen. Ein offener Create-Entwurf bleibt erhalten und wird
 * nach der Anmeldung wieder geöffnet.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouter } from "@tanstack/react-router";
import { useEffect } from "react";

import { SESSION_QUERY_KEY, type Session } from "@/app/auth";
import { draftScope, markCreateReopen } from "@/features/issues/create-store";
import { setUnauthenticatedHandler } from "@/lib/graphql";

export function useSessionExpiry(): void {
  const navigate = useNavigate();
  const client = useQueryClient();
  const router = useRouter();
  useEffect(() => {
    let busy = false;
    setUnauthenticatedHandler(() => {
      const { pathname, searchStr, hash } = router.state.location;
      if (busy || pathname === "/login") return;
      busy = true;
      const session = client.getQueryData<Session | null>(SESSION_QUERY_KEY);
      if (session) markCreateReopen(draftScope(session.user.id, session.workspace.id));
      client.setQueryData(SESSION_QUERY_KEY, null);
      void navigate({
        to: "/login",
        search: { redirect: pathname + searchStr + hash },
        replace: true,
      }).finally(() => {
        busy = false;
      });
    });
    return () => setUnauthenticatedHandler(null);
  }, [client, navigate, router]);
}
