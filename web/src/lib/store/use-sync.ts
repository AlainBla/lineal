import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useSession } from "@/app/auth";

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
    });
    client.start();
    return () => client.stop();
  }, [enabled, qc, userId, workspace]);
}
