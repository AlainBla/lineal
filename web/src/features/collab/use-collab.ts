/** React-Anbindung des Collab-Providers: ein Provider je Dokument, Neuaufbau bei Epochenwechsel. */
import { useEffect, useState, useSyncExternalStore } from "react";

import { type CollabOptions, CollabProvider, type CollabStatus, collabUrl } from "./provider";

export interface CollabSession {
  provider: CollabProvider | null;
  status: CollabStatus;
  canWrite: boolean;
  synced: boolean;
  /** Schon einmal verbunden gewesen. */
  everConnected: boolean;
  /** Erhöht sich bei jedem Neuaufbau (als `key` für den Editor verwenden). */
  generation: number;
}

interface Snapshot {
  status: CollabStatus;
  canWrite: boolean;
  synced: boolean;
  everConnected: boolean;
}

/** Stabiler Snapshot je Provider (für `useSyncExternalStore`). */
const snapshots = new WeakMap<CollabProvider, Snapshot>();

function snapshotOf(p: CollabProvider): Snapshot {
  const prev = snapshots.get(p);
  if (
    prev &&
    prev.status === p.status &&
    prev.canWrite === p.canWrite &&
    prev.synced === p.synced &&
    prev.everConnected === p.everConnected
  ) {
    return prev;
  }
  const next = {
    status: p.status,
    canWrite: p.canWrite,
    synced: p.synced,
    everConnected: p.everConnected,
  };
  snapshots.set(p, next);
  return next;
}

const IDLE: Snapshot = {
  status: "connecting",
  canWrite: false,
  synced: false,
  everConnected: false,
};

export function useCollab(
  documentId: string,
  options?: Partial<CollabOptions>,
  enabled = true,
): CollabSession {
  const [generation, setGeneration] = useState(0);
  const [provider, setProvider] = useState<CollabProvider | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const p = new CollabProvider({ url: collabUrl(documentId), ...options });
    const off = p.onReset(() => setGeneration((g) => g + 1));
    // Der Provider entsteht (wie jede Verbindung) erst im Effekt, nicht beim Rendern.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setProvider(p);
    p.connect();
    return () => {
      off();
      p.destroy();
      setProvider((cur) => (cur === p ? null : cur));
    };
    // `options` ist nur für Tests gedacht und ändert sich nicht.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [documentId, generation, enabled]);

  const snap = useSyncExternalStore(
    (cb) => (provider ? provider.subscribe(cb) : () => {}),
    () => (provider ? snapshotOf(provider) : IDLE),
  );
  return { provider, generation, ...snap };
}
