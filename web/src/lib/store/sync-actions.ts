/**
 * Anwenden von SyncActions (docs/09 §5) auf die TanStack-Query-Caches der Feature-Hooks.
 *
 * Der Server liefert je Aktion `{ id, action, modelName, modelId, data }`. Issues kommen in der
 * Form der GraphQL-Felder (`ISSUE_FIELDS`) und werden direkt in die Listen- und Detail-Caches
 * gepatcht; für alle anderen Modelle (und für Änderungen, die Listenzugehörigkeit oder
 * Kennungen betreffen) werden gezielt die Query-Keys des jeweiligen Bereichs invalidiert.
 * Invalidierungen eines Pakets werden zusammengefasst (jeder Key höchstens einmal).
 *
 * Cache-Schlüssel der Issues enthalten teils den Team-Key (`["issues","team","ENG"]`), teils die
 * UUID (`["issue", "<uuid>"]` / `["issue", "ENG-1"]`). Gepatcht wird deshalb immer über die
 * Issue-`id` in allen Einträgen unter `["issues"]` und `["issue"]` (`patchIssueCaches`), nie über
 * einen konstruierten Key.
 */
import type { QueryClient, QueryKey } from "@tanstack/react-query";

import { patchIssueCaches } from "@/features/issues/hooks";
import type { IssuePatch } from "@/features/issues/model";
import { keys as issueKeys, type IssueList } from "@/features/issues/queries";
import { inboxKeys } from "@/features/inbox/queries";
import type { Issue } from "@/gql/graphql";

/** `I` insert, `U` update, `A` archive, `V` unarchive, `D` delete, `C` covering, `G` Gruppenwechsel. */
export type SyncActionKind = "I" | "U" | "A" | "V" | "D" | "C" | "G";

export interface SyncAction {
  /** Stream-Cursor (`lastSyncId`); streng monoton, Lücken sind normal. */
  id: number;
  action: SyncActionKind;
  modelName: string;
  modelId: string;
  data: Record<string, unknown>;
  syncGroups?: string[];
  syncId?: number;
  transactionId?: string | null;
}

/** Felder, deren Änderung die Zugehörigkeit zu gefilterten Listen (Team, „My issues“, Projekt …) verändern kann. */
const MEMBERSHIP_FIELDS = [
  "teamId",
  "assigneeId",
  "projectId",
  "cycleId",
  "projectMilestoneId",
  "parentId",
  "stateType",
  "archivedAt",
  "trashed",
] as const;

/** Sammelt Invalidierungen eines Pakets ohne Duplikate. */
class Pending {
  private readonly seen = new Map<string, QueryKey>();
  private all = false;

  add(key: QueryKey): void {
    this.seen.set(JSON.stringify(key), key);
  }

  everything(): void {
    this.all = true;
  }

  flush(qc: QueryClient): void {
    if (this.all) {
      void qc.invalidateQueries();
      return;
    }
    for (const queryKey of this.seen.values()) void qc.invalidateQueries({ queryKey });
  }
}

/** Alle gecachten Vorkommen des Issues (Listen und Detail). */
function cachedIssues(qc: QueryClient, id: string): Issue[] {
  const found: Issue[] = [];
  for (const [, list] of qc.getQueriesData<IssueList>({ queryKey: issueKeys.issues })) {
    if (list && Array.isArray(list.issues)) {
      const hit = list.issues.find((i) => i.id === id);
      if (hit) found.push(hit);
    }
  }
  for (const [, one] of qc.getQueriesData<Issue>({ queryKey: ["issue"] })) {
    if (one && one.id === id) found.push(one);
  }
  return found;
}

/** Entfernt das Issue aus allen Listen-Caches. */
function removeFromLists(qc: QueryClient, id: string): void {
  qc.setQueriesData<IssueList>({ queryKey: issueKeys.issues }, (old) =>
    old && Array.isArray(old.issues)
      ? { ...old, issues: old.issues.filter((i) => i.id !== id) }
      : old,
  );
}

function str(v: unknown): string | undefined {
  return typeof v === "string" && v ? v : undefined;
}

function applyIssue(qc: QueryClient, a: SyncAction, pending: Pending): void {
  const id = a.modelId;
  pending.add(["overview"]);
  // Verlauf, Kinder und Relationen ändern sich bei jeder Issue-Änderung (auch Kinder/Gegenstücke).
  pending.add(issueKeys.issueDetails);
  if (a.action === "A" || a.action === "D") {
    removeFromLists(qc, id);
    pending.add(["issue"]);
    pending.add(issueKeys.issues);
    return;
  }
  const before = cachedIssues(qc, id);
  patchIssueCaches(qc, id, a.data as IssuePatch);
  // Neue/wiederhergestellte Issues, unbekannte Issues und Änderungen, die Listen-Zugehörigkeit
  // oder Kennung betreffen, brauchen frische Listen; reine Feldänderungen sind mit dem Patch erledigt.
  const moved =
    a.action === "I" ||
    a.action === "V" ||
    before.length === 0 ||
    before.some(
      (old) =>
        MEMBERSHIP_FIELDS.some(
          (f) => f in a.data && (old as unknown as Record<string, unknown>)[f] !== a.data[f],
        ) ||
        (str(a.data.identifier) !== undefined && old.identifier !== a.data.identifier),
    );
  if (moved) {
    pending.add(issueKeys.issues);
    pending.add(["issue"]);
  }
}

/** Welche Query-Keys ein Modell außer Issues betrifft. */
function invalidateFor(a: SyncAction, pending: Pending): void {
  const data = a.data;
  switch (a.modelName) {
    case "Notification":
      pending.add(inboxKeys.all);
      pending.add(["sidebar-data"]);
      break;
    case "Comment": {
      const issueId = str(data.issueId);
      pending.add(issueId ? issueKeys.comments(issueId) : ["comments"]);
      pending.add(["planning"]);
      pending.add(["docs"]);
      break;
    }
    case "Reaction":
      pending.add(["comments"]);
      break;
    case "Attachment":
    case "IssueRelation":
    case "IssueSubscription":
      pending.add(issueKeys.issueDetails);
      pending.add(["issue"]);
      break;
    case "Project":
    case "ProjectMilestone":
    case "ProjectUpdate":
    case "Initiative":
    case "InitiativeUpdate":
      pending.add(["planning"]);
      pending.add(["overview"]);
      break;
    case "Cycle":
      pending.add(["planning"]);
      pending.add(["team-meta"]);
      pending.add(["overview"]);
      break;
    case "Document":
      pending.add(["docs"]);
      pending.add(["planning"]);
      break;
    case "WorkflowState":
    case "IssueLabel":
      // Umbenennungen ändern `stateName` o. Ä. der gecachten Issues.
      pending.add(["team-meta"]);
      pending.add(["settings"]);
      pending.add(issueKeys.issues);
      pending.add(["issue"]);
      break;
    case "Template":
      pending.add(["settings"]);
      break;
    case "Team":
      pending.add(["team-meta"]);
      pending.add(["settings"]);
      pending.add(["sidebar-data"]);
      pending.add(issueKeys.teams);
      pending.add(["session"]);
      break;
    case "CustomView":
    case "Favorite":
      pending.add(["views"]);
      break;
    case "User":
      pending.add(["users"]);
      pending.add(["known-users"]);
      pending.add(["settings"]);
      break;
    case "Organization":
      pending.add(["session"]);
      pending.add(["settings"]);
      break;
    default:
      break;
  }
}

/** Wendet ein Paket von Aktionen in Reihenfolge an und invalidiert gezielt (zusammengefasst). */
export function applySyncActions(qc: QueryClient, actions: readonly SyncAction[]): void {
  const pending = new Pending();
  for (const a of actions) {
    if (a.modelName === "TeamMembership") {
      // Zugriff auf Teams hat sich geändert (neu oder entzogen): alles neu laden, auch die Session.
      pending.everything();
    } else if (a.action === "G" || a.action === "C") {
      pending.everything();
    } else if (a.modelName === "Issue") {
      applyIssue(qc, a, pending);
    } else {
      invalidateFor(a, pending);
    }
  }
  pending.flush(qc);
}

/** Wendet eine einzelne Aktion an. */
export function applySyncAction(qc: QueryClient, action: SyncAction): void {
  applySyncActions(qc, [action]);
}
