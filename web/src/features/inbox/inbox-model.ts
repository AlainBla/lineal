/**
 * Reine Logik der Inbox-Anzeige (F.09, docs/05 §6.6/§6.7): Gruppen je `groupKey`, Sortierung,
 * Priority-Tabs und Filter. Display options liegen pro Browser im localStorage.
 */
import { useSyncExternalStore } from "react";

import type { Notification } from "@/gql/graphql";

export type InboxOrdering = "newest" | "oldest" | "priority";
export type InboxGrouping = "none" | "focus";
export type InboxTab = "priority" | "other";

export interface InboxPrefs {
  ordering: InboxOrdering;
  /** `focus`: ungelesene Gruppen stehen in einem eigenen Abschnitt oben. */
  groupUnreads: InboxGrouping;
  unreadFirst: boolean;
  /** Priority inbox: Tabs „Priority“ und „Other“. */
  priorityInbox: boolean;
}

export const DEFAULT_PREFS: InboxPrefs = {
  ordering: "newest",
  groupUnreads: "none",
  unreadFirst: false,
  priorityInbox: false,
};

export interface InboxFilters {
  /** Kategorien (`Notification.category`); leer = alle. */
  categories: string[];
  /** Akteure (`actorId`); leer = alle. */
  actorIds: string[];
}

export const NO_FILTERS: InboxFilters = { categories: [], actorIds: [] };

/** Kategorien des Filters „Notification type“ (docs/05 §6.7). */
export const CATEGORY_LABELS: Array<[string, string]> = [
  ["assignments", "Assignments"],
  ["commentsAndReplies", "Comments and replies"],
  ["mentions", "Mentions"],
  ["reactions", "Reactions"],
  ["reminders", "Reminders and deadlines"],
  ["statusChanges", "Status changes"],
  ["subscriptions", "Subscriptions"],
  ["triage", "Triage"],
  ["postsAndUpdates", "Updates"],
  ["documentChanges", "Document changes"],
  ["system", "System"],
];

export function filterCount(f: InboxFilters): number {
  return f.categories.length + f.actorIds.length;
}

export function applyFilters(list: Notification[], f: InboxFilters): Notification[] {
  return list.filter(
    (n) =>
      (f.categories.length === 0 || f.categories.includes(n.category)) &&
      (f.actorIds.length === 0 || (n.actorId !== null && f.actorIds.includes(n.actorId))),
  );
}

/** Eine Zeile der Inbox: alle Notifications eines Bezugsobjekts (`groupKey`). */
export interface InboxGroup {
  key: string;
  /** Neueste Notification der Gruppe (zeigt Text und Typ). */
  latest: Notification;
  items: Notification[];
  unread: number;
  isPriority: boolean;
}

const stamp = (n: Notification) => Date.parse(n.updatedAt || n.createdAt) || 0;

/** Fasst Notifications nach `groupKey` zusammen (Reihenfolge der Eingabe bleibt je Gruppe erhalten). */
export function groupNotifications(list: Notification[]): InboxGroup[] {
  const map = new Map<string, Notification[]>();
  for (const n of list) {
    const key = n.groupKey || n.id;
    const items = map.get(key);
    if (items) items.push(n);
    else map.set(key, [n]);
  }
  return [...map.entries()].map(([key, items]) => {
    const latest = items.reduce((a, b) => (stamp(b) > stamp(a) ? b : a));
    return {
      key,
      latest,
      items,
      unread: items.filter((n) => !n.readAt).length,
      isPriority: items.some((n) => n.isPriority),
    };
  });
}

export function orderGroups(groups: InboxGroup[], prefs: InboxPrefs): InboxGroup[] {
  const out = [...groups];
  out.sort((a, b) => {
    if (prefs.unreadFirst && a.unread > 0 !== b.unread > 0) return a.unread > 0 ? -1 : 1;
    if (prefs.ordering === "priority" && a.isPriority !== b.isPriority)
      return a.isPriority ? -1 : 1;
    const d = stamp(b.latest) - stamp(a.latest);
    return prefs.ordering === "oldest" ? -d : d;
  });
  return out;
}

export function splitByTab(groups: InboxGroup[], tab: InboxTab): InboxGroup[] {
  return groups.filter((g) => (tab === "priority" ? g.isPriority : !g.isPriority));
}

/** Pipeline der Anzeige: Filter → Gruppen → Tab (nur bei Priority inbox) → Sortierung. */
export function buildInbox(
  list: Notification[],
  prefs: InboxPrefs,
  filters: InboxFilters,
  tab: InboxTab,
): { groups: InboxGroup[]; priorityCount: number; otherCount: number } {
  const all = groupNotifications(applyFilters(list, filters));
  const priorityCount = all.filter((g) => g.isPriority).length;
  const shown = prefs.priorityInbox ? splitByTab(all, tab) : all;
  return {
    groups: orderGroups(shown, prefs),
    priorityCount,
    otherCount: all.length - priorityCount,
  };
}

// ---------- Display options (localStorage) ----------

const PREFS_KEY = "lineal-inbox-prefs";
let prefs: InboxPrefs = loadPrefs();
const listeners = new Set<() => void>();

function loadPrefs(): InboxPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return DEFAULT_PREFS;
    const p = JSON.parse(raw) as Partial<InboxPrefs>;
    return {
      ordering: p.ordering === "oldest" || p.ordering === "priority" ? p.ordering : "newest",
      groupUnreads: p.groupUnreads === "focus" ? "focus" : "none",
      unreadFirst: p.unreadFirst === true,
      priorityInbox: p.priorityInbox === true,
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

export function getInboxPrefs(): InboxPrefs {
  return prefs;
}

export function setInboxPrefs(patch: Partial<InboxPrefs>) {
  prefs = { ...prefs, ...patch };
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    /* ignorieren */
  }
  listeners.forEach((l) => l());
}

export function useInboxPrefs(): InboxPrefs {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    getInboxPrefs,
    getInboxPrefs,
  );
}
