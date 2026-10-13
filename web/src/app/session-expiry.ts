/**
 * Abgelaufene Session (HTTP 401 auf GraphQL): zurück zum Login mit Rücksprungadresse, statt die
 * Seite durch eine Fehlerkarte zu ersetzen. Ein offener Create-Entwurf bleibt erhalten und wird
 * nach der Anmeldung wieder geöffnet.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { markCreateReopen } from "@/features/issues/create-store";
import { setUnauthenticatedHandler } from "@/lib/graphql";

export function useSessionExpiry(): void {
  const navigate = useNavigate();
  const client = useQueryClient();
  useEffect(() => {
    let busy = false;
    setUnauthenticatedHandler(() => {
      const { pathname, search, hash } = window.location;
      if (busy || pathname === "/login") return;
      busy = true;
      markCreateReopen();
      client.setQueryData(SESSION_QUERY_KEY, null);
      void navigate({
        to: "/login",
        search: { redirect: pathname + search + hash },
        replace: true,
      }).finally(() => {
        busy = false;
      });
    });
    return () => setUnauthenticatedHandler(null);
  }, [client, navigate]);
}
