/**
 * Daten der Inbox: GraphQL-Operationen, Query-Keys und Hooks (WP15 ersetzt die Datenquelle hinter diesen Hooks).
 * Mutationen aktualisieren den Listen-Cache optimistisch (Rollback bei Fehler) und den Unread-Badge der Sidebar.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { Notification, NotificationConnection } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

export const inboxKeys = {
  all: ["inbox"] as const,
  list: (unreadOnly: boolean, showSnoozed: boolean) =>
    ["inbox", "list", unreadOnly, showSnoozed] as const,
};

const FIELDS = `id type category title subtitle url groupKey actorId actorName issueId issueIdentifier
  projectId initiativeId documentId commentId projectMilestoneId projectUpdateId initiativeUpdateId bundledTypes
  isPriority readAt snoozedUntilAt unsnoozedAt archivedAt createdAt updatedAt`;

export interface InboxFilter {
  unreadOnly: boolean;
  showSnoozed: boolean;
}

export async function fetchNotifications(
  f: InboxFilter,
  signal?: AbortSignal,
): Promise<Notification[]> {
  const res = await gqlRequest<
    { notifications: NotificationConnection },
    { unreadOnly: boolean; includeSnoozed: boolean; first: number }
  >(
    `query InboxNotifications($unreadOnly: Boolean, $includeSnoozed: Boolean, $first: Int) {
      notifications(unreadOnly: $unreadOnly, includeSnoozed: $includeSnoozed, first: $first) {
        nodes { ${FIELDS} } pageInfo { hasNextPage endCursor }
      }
    }`,
    { unreadOnly: f.unreadOnly, includeSnoozed: f.showSnoozed, first: 100 },
    { signal },
  );
  return res.notifications.nodes;
}

export function useNotifications(f: InboxFilter) {
  return useQuery({
    queryKey: inboxKeys.list(f.unreadOnly, f.showSnoozed),
    queryFn: ({ signal }) => fetchNotifications(f, signal),
  });
}

export type InboxAction =
  | { kind: "read"; id: string; read: boolean }
  | { kind: "snooze"; id: string; until: string }
  | { kind: "archive"; id: string }
  | { kind: "markAllRead" }
  | { kind: "archiveAll"; filter: "all" | "read" | "completed" };

async function run(a: InboxAction): Promise<void> {
  switch (a.kind) {
    case "read":
      await gqlRequest(
        `mutation InboxMarkRead($id: String!, $input: NotificationUpdateInput!) {
          notificationUpdate(id: $id, input: $input) { success }
        }`,
        { id: a.id, input: { read: a.read } },
      );
      return;
    case "snooze":
      await gqlRequest(
        `mutation InboxSnooze($id: String!, $input: NotificationUpdateInput!) {
          notificationUpdate(id: $id, input: $input) { success }
        }`,
        { id: a.id, input: { snoozedUntilAt: a.until } },
      );
      return;
    case "archive":
      await gqlRequest(
        `mutation InboxArchive($id: String!) { notificationArchive(id: $id, archive: true) { success } }`,
        { id: a.id },
      );
      return;
    case "markAllRead":
      await gqlRequest(
        `mutation InboxMarkAllRead { notificationMarkReadAll(filter: "all") { success count } }`,
      );
      return;
    case "archiveAll":
      await gqlRequest(
        `mutation InboxArchiveAll($filter: String) { notificationArchiveAll(filter: $filter) { success count } }`,
        { filter: a.filter },
      );
  }
}

/** Wendet die Aktion auf eine Notification-Liste an (optimistisch, rein). */
export function applyInboxAction(
  list: Notification[],
  a: InboxAction,
  now: string,
): Notification[] {
  switch (a.kind) {
    case "read": {
      // Der Server setzt gelesen/ungelesen für die ganze Gruppe (docs/05 §6.8).
      const key = list.find((n) => n.id === a.id)?.groupKey;
      return list.map((n) =>
        n.id === a.id || (key && n.groupKey === key) ? { ...n, readAt: a.read ? now : null } : n,
      );
    }
    case "snooze":
    case "archive": {
      // Snooze und Löschen gelten für die ganze Gruppe (docs/05 §6.4/§6.8).
      const key = list.find((n) => n.id === a.id)?.groupKey;
      return list.filter((n) => n.id !== a.id && (!key || n.groupKey !== key));
    }
    case "markAllRead":
      return list.map((n) => (n.readAt ? n : { ...n, readAt: now }));
    case "archiveAll":
      // "completed" kennt nur der Server (Issue geschlossen); die Liste lädt danach neu.
      if (a.filter === "completed") return list;
      return a.filter === "all" ? [] : list.filter((n) => !n.readAt);
  }
}

export function useInboxAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: run,
    onMutate: async (a) => {
      await qc.cancelQueries({ queryKey: inboxKeys.all });
      const snapshot = qc.getQueriesData<Notification[]>({ queryKey: inboxKeys.all });
      const now = new Date().toISOString();
      qc.setQueriesData<Notification[]>({ queryKey: inboxKeys.all }, (old) =>
        old ? applyInboxAction(old, a, now) : old,
      );
      return { snapshot };
    },
    onError: (_e, _a, ctx) => ctx?.snapshot.forEach(([k, d]) => qc.setQueryData(k, d)),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: inboxKeys.all });
      // Unread-Badge der Sidebar.
      void qc.invalidateQueries({ queryKey: ["sidebar-data"] });
    },
  });
}

/** Start des nächsten Cycles des Teams (für „Next cycle“, docs/05 §6.7); `null` ohne künftigen Cycle. */
export function useNextCycleStart(teamKey: string | null, enabled: boolean) {
  return useQuery({
    queryKey: ["inbox", "next-cycle", teamKey] as const,
    enabled: enabled && !!teamKey,
    staleTime: 60_000,
    queryFn: async (): Promise<Date | null> => {
      const res = await gqlRequest<
        { cycles: Array<{ startsAt: string }> },
        { teamId: string; type: string }
      >(
        `query InboxNextCycle($teamId: String!, $type: String) { cycles(teamId: $teamId, type: $type) { startsAt } }`,
        {
          teamId: teamKey ?? "",
          type: "next",
        },
      );
      const start = res.cycles
        .map((c) => new Date(c.startsAt))
        .find((d) => d.getTime() > Date.now());
      return start ?? null;
    },
  });
}

/** Snooze-Ziele (docs/08 §2.9); `now` injizierbar für Tests. */
export function snoozePresets(
  now: Date,
  nextCycleStart?: Date | null,
): Array<{ id: string; label: string; until: Date }> {
  const at = (d: Date, h: number) => {
    const x = new Date(d);
    x.setHours(h, 0, 0, 0);
    return x;
  };
  const tomorrow = new Date(now.getTime() + 24 * 3600_000);
  const nextWeek = new Date(now.getTime() + 7 * 24 * 3600_000);
  const month = new Date(now);
  month.setMonth(month.getMonth() + 1);
  // Heute Abend 18:00; ab 18 Uhr morgen Abend.
  const evening = at(now, 18);
  if (evening.getTime() <= now.getTime()) evening.setDate(evening.getDate() + 1);
  return [
    { id: "1h", label: "An hour from now", until: new Date(now.getTime() + 3600_000) },
    { id: "3h", label: "In 3 hours", until: new Date(now.getTime() + 3 * 3600_000) },
    { id: "evening", label: "This evening", until: evening },
    { id: "tomorrow", label: "Tomorrow", until: at(tomorrow, 9) },
    { id: "week", label: "Next week", until: at(nextWeek, 9) },
    { id: "month", label: "A month from now", until: at(month, 9) },
    ...(nextCycleStart && nextCycleStart.getTime() > now.getTime()
      ? [{ id: "cycle", label: "Next cycle", until: nextCycleStart }]
      : []),
  ];
}
