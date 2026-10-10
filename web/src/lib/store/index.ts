// Sync-Store (WP15): WebSocket-Strom `/sync` → TanStack-Query-Caches der Feature-Hooks.
export { applySyncAction, applySyncActions } from "./sync-actions";
export type { SyncAction, SyncActionKind } from "./sync-actions";
export { defaultSyncUrl, localStorageSync, SyncClient } from "./sync-client";
export type {
  ServerMessage,
  SocketLike,
  SyncClientOptions,
  SyncStatus,
  SyncStorage,
} from "./sync-client";
export { useSyncStream } from "./use-sync";
