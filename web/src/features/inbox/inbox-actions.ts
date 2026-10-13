/**
 * Kontextmenü-Aktionen einer Inbox-Zeile (F.09, docs/05 §6.8): Unsubscribe und Favorite wirken auf das
 * Bezugsobjekt der Notification, nicht auf die Notification selbst.
 */
import type { Notification } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";

import { targetKind } from "./notification-types";

/** Beendet das auslösende Abo (Issue-, Dokument-, Projekt- oder Initiativen-Abo). */
export async function unsubscribeFrom(n: Notification): Promise<void> {
  if (n.issueId || n.issueIdentifier) {
    await gqlRequest(
      `mutation InboxIssueUnsubscribe($id: String!) { issueUnsubscribe(id: $id) { success } }`,
      { id: n.issueIdentifier ?? n.issueId },
    );
  } else if (n.documentId) {
    await gqlRequest(
      `mutation InboxDocUnsubscribe($id: String!) { documentUnsubscribe(id: $id) { success } }`,
      { id: n.documentId },
    );
  } else if (n.projectId || n.initiativeId) {
    await gqlRequest(
      `mutation InboxObjectUnsubscribe($input: NotificationSubscriptionInput!) {
        notificationSubscriptionUpdate(input: $input) { id active }
      }`,
      {
        input: n.projectId
          ? { targetType: "project", targetId: n.projectId, active: false }
          : { targetType: "initiative", targetId: n.initiativeId, active: false },
      },
    );
  } else {
    throw new Error("This notification has no subscription to end.");
  }
}

/** Favorit-Typ und Ziel-ID des Bezugsobjekts; `null` bei System-Benachrichtigungen. */
export function favoriteTarget(n: Notification): { type: string; targetId: string } | null {
  switch (targetKind(n)) {
    case "issue":
      return n.issueId ? { type: "issue", targetId: n.issueId } : null;
    case "document":
      return n.documentId ? { type: "document", targetId: n.documentId } : null;
    case "initiative":
      return n.initiativeId ? { type: "initiative", targetId: n.initiativeId } : null;
    case "milestone":
    case "project":
      return n.projectId ? { type: "project", targetId: n.projectId } : null;
    default:
      return null;
  }
}

export async function favoriteObject(n: Notification): Promise<void> {
  const t = favoriteTarget(n);
  if (!t) throw new Error("This notification has no object to favorite.");
  await gqlRequest(
    `mutation InboxFavorite($input: FavoriteCreateInput!) { favoriteCreate(input: $input) { success } }`,
    { input: t },
  );
}
