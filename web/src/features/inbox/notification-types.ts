/**
 * Darstellung der Notification-Typen (docs/05 §6.2) und der 1-Minuten-Bündelung (§6.6).
 * Unbekannte Typen werden generisch aus dem camelCase-Namen lesbar gemacht (Clients müssen sie tolerieren).
 */
import type { Notification } from "@/gql/graphql";

/** Kurze, englische Bezeichnung je Typ (Badge und Bündel-Detailzeile). */
const LABELS: Record<string, string> = {
  issueAssignedToYou: "Assigned to you",
  issueUnassignedFromYou: "Unassigned",
  issueMention: "Mention",
  issueCommentMention: "Comment mention",
  issueNewComment: "Comment",
  issueThreadResolved: "Thread resolved",
  issueCommentReaction: "Reaction",
  issueEmojiReaction: "Reaction",
  issueStatusChanged: "Status changed",
  issueStatusChangedAll: "Status changed",
  issueReopened: "Reopened",
  issuePriorityUrgent: "Urgent",
  issueBlocking: "Blocked",
  issueUnblocked: "Unblocked",
  issueDue: "Due soon",
  issueSlaHighRisk: "SLA at risk",
  issueSlaBreached: "SLA breached",
  issueReminder: "Reminder",
  issueSubscribed: "Subscribed",
  issueUnsubscribed: "Unsubscribed",
  issueCreated: "New issue",
  issueAddedToView: "Added to view",
  issueAddedToTriage: "Triage",
  projectUpdateCreated: "Project update",
  projectNewComment: "Project comment",
  projectCommentMention: "Project mention",
  projectMilestoneNewComment: "Milestone comment",
  projectMilestoneCommentMention: "Milestone mention",
  projectUpdateNewComment: "Update comment",
  projectUpdateCommentMention: "Update mention",
  initiativeNewComment: "Initiative comment",
  initiativeCommentMention: "Initiative mention",
  initiativeUpdateCreated: "Initiative update",
  initiativeUpdateNewComment: "Update comment",
  initiativeUpdateCommentMention: "Update mention",
  documentNewComment: "Document comment",
  documentCommentMention: "Document mention",
  documentSubscribed: "Subscribed",
  documentUnsubscribed: "Unsubscribed",
};

/** `projectUpdateNewComment` → „Project update new comment“ (Rückfall für unbekannte Typen). */
function humanize(type: string): string {
  const words = type
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .trim()
    .toLowerCase();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : "Notification";
}

export function typeLabel(type: string): string {
  return LABELS[type] ?? humanize(type);
}

/** Bezugsobjekt der Notification für die Anzeige (Meilenstein und Berichte gruppieren unter ihrem Projekt). */
export function targetKind(
  n: Pick<
    Notification,
    | "issueId"
    | "documentId"
    | "initiativeId"
    | "projectId"
    | "projectMilestoneId"
    | "projectUpdateId"
    | "initiativeUpdateId"
  >,
): "issue" | "milestone" | "project" | "initiative" | "document" | "system" {
  if (n.issueId) return "issue";
  if (n.documentId) return "document";
  if (n.projectMilestoneId) return "milestone";
  if (n.projectId) return "project";
  if (n.initiativeId) return "initiative";
  return "system";
}

export interface BundleSummary {
  /** Zahl der zusammengefassten Ereignisse (1 = nicht gebündelt). */
  count: number;
  /** Eindeutige Bezeichnungen der Änderungen in Reihenfolge des Auftretens. */
  labels: string[];
}

/** Bündel einer Notification: `bundledTypes` listet alle Ereignisse inkl. des Haupttyps. */
export function bundleSummary(n: Pick<Notification, "type" | "bundledTypes">): BundleSummary {
  const types = n.bundledTypes ?? [];
  if (types.length <= 1) return { count: 1, labels: [typeLabel(n.type)] };
  const labels: string[] = [];
  for (const t of types) {
    const l = typeLabel(t);
    if (!labels.includes(l)) labels.push(l);
  }
  return { count: types.length, labels };
}
