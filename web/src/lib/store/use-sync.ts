import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useSession } from "@/app/auth";

import { usersKey, type DirectoryUser } from "@/features/issues/users";

import { fetchBootstrap } from "./bootstrap";
import { defaultSyncUrl, localStorageSync, SyncClient } from "./sync-client";

/**
 * Hält den Echtzeit-Strom (`/sync`) offen, solange eine Session besteht, und wendet die Aktionen
 * auf die Query-Caches an. Einmal in der Shell einhängen. In Tests (`MODE === "test"`) inaktiv,
 * außer `enabled` wird ausdrücklich gesetzt.
 */
export function useSyncStream(enabled: boolean = import.meta.env.MODE !== "test"): void {
  const qc = useQueryClient();
  const session = useSession();
  const userId = session?.user.id;
  const workspace = session?.workspace.urlKey;

  useEffect(() => {
    if (!enabled || !userId || typeof WebSocket === "undefined") return;
    const client = new SyncClient({
      qc,
      url: defaultSyncUrl(),
      storage: localStorageSync(`${workspace ?? ""}:${userId}`),
      // Erststart: das Personenverzeichnis kommt aus dem Bootstrap (spart die Abfrage `users`).
      bootstrap: async () => {
        const res = await fetchBootstrap(["User"]);
        const users: DirectoryUser[] = res.objects
          .filter((o) => o.__class === "User" && typeof o.name === "string")
          .map((o) => ({
            id: o.id,
            name: String(o.name),
            displayName: typeof o.displayName === "string" ? o.displayName : "",
          }));
        qc.setQueryData(usersKey({ userId, ws: workspace ?? "" }), users);
        return res.lastSyncId;
      },
    });
    client.start();
    return () => client.stop();
  }, [enabled, qc, userId, workspace]);
}
