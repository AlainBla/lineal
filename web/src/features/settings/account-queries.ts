/** Daten des persönlichen Accounts: Profil, Passwort, Sessions, Benachrichtigungen (WP29). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { SESSION_QUERY_KEY } from "@/app/auth";
import { gqlRequest } from "@/lib/graphql";

export interface Viewer {
  id: string;
  name: string;
  displayName: string;
  email: string;
  role: string;
  avatarUrl: string | null;
  timezone: string | null;
  createdAt: string;
}

const VIEWER_FIELDS = "id name displayName email role avatarUrl timezone createdAt";
export const viewerKey = ["account", "viewer"] as const;

export function useViewer() {
  return useQuery({
    queryKey: viewerKey,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ viewer: Viewer }>(
          `query AccountViewer { viewer { ${VIEWER_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).viewer,
  });
}

/** Fehlend = unverändert, `null` = leeren. */
export interface ViewerUpdate {
  name?: string;
  displayName?: string;
  avatarUrl?: string | null;
  timezone?: string | null;
}

export function useUpdateViewer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: ViewerUpdate) =>
      (
        await gqlRequest<{ viewerUpdate: { viewer: Viewer } }, { input: ViewerUpdate }>(
          `mutation AccountViewerUpdate($input: ViewerUpdateInput!) {
            viewerUpdate(input: $input) { success viewer { ${VIEWER_FIELDS} } }
          }`,
          { input },
        )
      ).viewerUpdate.viewer,
    onSuccess: (viewer) => {
      qc.setQueryData(viewerKey, viewer);
      // Sidebar und Menüs zeigen Name und Avatar aus der Session.
      void qc.invalidateQueries({ queryKey: SESSION_QUERY_KEY });
    },
  });
}

/** IANA-Zeitzonen des Browsers; ohne Unterstützung eine kurze Auswahl. */
export function timezoneOptions(): string[] {
  const intl = Intl as unknown as { supportedValuesOf?: (k: string) => string[] };
  try {
    const all = intl.supportedValuesOf?.("timeZone");
    if (all && all.length > 0) return all;
  } catch {
    /* ältere Browser */
  }
  return ["UTC", "Europe/Berlin", "Europe/London", "America/New_York", "America/Los_Angeles"];
}

export const AVATAR_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];
export const AVATAR_MAX_BYTES = 5 * 1024 * 1024;

/** Prüft die Datei vor dem Hochladen; liefert eine Fehlermeldung oder `null`. */
export function avatarProblem(file: Pick<File, "type" | "size">): string | null {
  if (!AVATAR_TYPES.includes(file.type)) return "Use a PNG, JPEG, GIF or WebP image.";
  if (file.size > AVATAR_MAX_BYTES) return "The image must be at most 5 MB.";
  return null;
}

/** Lädt ein Bild über den vorhandenen Upload-Weg (`fileUpload` + signierter PUT); liefert die `assetUrl`. */
export async function uploadAvatar(file: File): Promise<string> {
  const data = await gqlRequest<
    {
      fileUpload: {
        uploadFile: {
          uploadUrl: string;
          assetUrl: string;
          headers: Array<{ key: string; value: string }>;
        };
      };
    },
    { filename: string; contentType: string; size: number }
  >(
    `mutation AccountFileUpload($filename: String!, $contentType: String!, $size: Int!) {
      fileUpload(filename: $filename, contentType: $contentType, size: $size) {
        success uploadFile { uploadUrl assetUrl headers { key value } }
      }
    }`,
    { filename: file.name, contentType: file.type, size: file.size },
  );
  const target = data.fileUpload.uploadFile;
  const res = await fetch(target.uploadUrl, {
    method: "PUT",
    headers: Object.fromEntries(target.headers.map((h) => [h.key, h.value])),
    body: file,
  });
  if (!res.ok) throw new Error("The image could not be uploaded.");
  return target.assetUrl;
}

export function usePasswordChange() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { currentPassword: string; newPassword: string }) =>
      (
        await gqlRequest<
          { passwordChange: { success: boolean; revokedSessions: number } },
          { currentPassword: string; newPassword: string }
        >(
          `mutation AccountPasswordChange($currentPassword: String!, $newPassword: String!) {
            passwordChange(currentPassword: $currentPassword, newPassword: $newPassword) {
              success revokedSessions
            }
          }`,
          v,
        )
      ).passwordChange,
    onSuccess: () => void qc.invalidateQueries({ queryKey: sessionsKey }),
  });
}

export interface UserSession {
  id: string;
  userAgent: string | null;
  ipAddress: string | null;
  createdAt: string;
  lastUsedAt: string | null;
  expiresAt: string;
  current: boolean;
}

export const sessionsKey = ["account", "sessions"] as const;

export function useSessions() {
  return useQuery({
    queryKey: sessionsKey,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ sessions: UserSession[] }>(
          `query AccountSessions {
            sessions { id userAgent ipAddress createdAt lastUsedAt expiresAt current }
          }`,
          undefined,
          { signal },
        )
      ).sessions,
  });
}

export function useRevokeSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      gqlRequest(
        `mutation AccountSessionRevoke($id: String!) { sessionRevoke(id: $id) { success } }`,
        { id },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: sessionsKey }),
  });
}

export function useRevokeOtherSessions() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () =>
      (
        await gqlRequest<{ sessionsRevokeOthers: { count: number } }>(
          `mutation AccountSessionsRevokeOthers { sessionsRevokeOthers { success count } }`,
        )
      ).sessionsRevokeOthers.count,
    onSuccess: () => void qc.invalidateQueries({ queryKey: sessionsKey }),
  });
}

/** „Chrome on Linux“ aus dem User-Agent; unbekannt → „Unknown device“. */
export function describeAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
  const browser = /Edg\//.test(ua)
    ? "Edge"
    : /OPR\//.test(ua)
      ? "Opera"
      : /Firefox\//.test(ua)
        ? "Firefox"
        : /Chrome\//.test(ua)
          ? "Chrome"
          : /Safari\//.test(ua)
            ? "Safari"
            : null;
  const os = /Android/.test(ua)
    ? "Android"
    : /iPhone|iPad/.test(ua)
      ? "iOS"
      : /Windows/.test(ua)
        ? "Windows"
        : /Mac OS X|Macintosh/.test(ua)
          ? "macOS"
          : /Linux|X11/.test(ua)
            ? "Linux"
            : null;
  if (browser && os) return `${browser} on ${os}`;
  return browser ?? os ?? "Unknown device";
}

// --- Benachrichtigungen --------------------------------------------------------------------

/** Kategorien nach docs/05 §6 (Reihenfolge der Anzeige). */
export const NOTIFICATION_CATEGORIES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "assignments", label: "Assignments" },
  { id: "mentions", label: "Mentions" },
  { id: "commentsAndReplies", label: "Comments and replies" },
  { id: "reactions", label: "Reactions" },
  { id: "statusChanges", label: "Status changes" },
  { id: "subscriptions", label: "Subscriptions" },
  { id: "triage", label: "Triage" },
  { id: "reminders", label: "Reminders and deadlines" },
  { id: "documentChanges", label: "Document changes" },
  { id: "postsAndUpdates", label: "Posts and updates" },
  { id: "system", label: "System" },
];

export interface NotificationSettings {
  emailEnabled: boolean;
  emailFormat: string;
  emailOnlyPriority: boolean;
  emailHoldOutsideWorkHours: boolean;
  emailImmediateUrgent: boolean;
  dueDateReminders: boolean;
  priorityCategories: string[];
  channels: Array<{ category: string; channel: string; enabled: boolean }>;
}

export type NotificationSettingsUpdate = Partial<Omit<NotificationSettings, "channels">> & {
  channels?: Array<{ category: string; channel: string; enabled: boolean }>;
};

const SETTINGS_FIELDS = `emailEnabled emailFormat emailOnlyPriority emailHoldOutsideWorkHours
  emailImmediateUrgent dueDateReminders priorityCategories channels { category channel enabled }`;
export const notificationSettingsKey = ["account", "notification-settings"] as const;

export function useNotificationSettings() {
  return useQuery({
    queryKey: notificationSettingsKey,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ notificationSettings: NotificationSettings }>(
          `query AccountNotificationSettings { notificationSettings { ${SETTINGS_FIELDS} } }`,
          undefined,
          { signal },
        )
      ).notificationSettings,
  });
}

export function useUpdateNotificationSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: NotificationSettingsUpdate) =>
      (
        await gqlRequest<
          { notificationSettingsUpdate: NotificationSettings },
          { input: NotificationSettingsUpdate }
        >(
          `mutation AccountNotificationSettingsUpdate($input: NotificationSettingsInput!) {
            notificationSettingsUpdate(input: $input) { ${SETTINGS_FIELDS} }
          }`,
          { input },
        )
      ).notificationSettingsUpdate,
    onSuccess: (data) => qc.setQueryData(notificationSettingsKey, data),
  });
}

/** Ob die Kategorie per E-Mail benachrichtigt (Default: nur `assignments` und `mentions`). */
export function emailEnabledFor(s: NotificationSettings, category: string): boolean {
  const row = s.channels.find((c) => c.category === category && c.channel === "email");
  return row ? row.enabled : category === "assignments" || category === "mentions";
}

export interface ObjectSubscription {
  id: string;
  targetType: string;
  targetId: string;
  contextViewType: string | null;
  notificationSubscriptionTypes: string[];
  includeSubInitiativeUpdates: boolean;
  active: boolean;
}

export const subscriptionsKey = ["account", "notification-subscriptions"] as const;

export function useNotificationSubscriptions() {
  return useQuery({
    queryKey: subscriptionsKey,
    queryFn: async ({ signal }) =>
      (
        await gqlRequest<{ notificationSubscriptions: ObjectSubscription[] }>(
          `query AccountNotificationSubscriptions {
            notificationSubscriptions {
              id targetType targetId contextViewType notificationSubscriptionTypes
              includeSubInitiativeUpdates active
            }
          }`,
          undefined,
          { signal },
        )
      ).notificationSubscriptions,
  });
}

/** Schaltet ein Objekt-Abo an/aus; die übrigen Felder bleiben unverändert. */
export function useToggleSubscription() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { sub: ObjectSubscription; active: boolean }) =>
      gqlRequest(
        `mutation AccountNotificationSubscriptionUpdate($input: NotificationSubscriptionInput!) {
          notificationSubscriptionUpdate(input: $input) { id active }
        }`,
        {
          input: {
            targetType: v.sub.targetType,
            targetId: v.sub.targetId,
            contextViewType: v.sub.contextViewType,
            notificationSubscriptionTypes: v.sub.notificationSubscriptionTypes,
            includeSubInitiativeUpdates: v.sub.includeSubInitiativeUpdates,
            active: v.active,
          },
        },
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: subscriptionsKey }),
  });
}
