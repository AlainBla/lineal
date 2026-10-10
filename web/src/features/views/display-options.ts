/**
 * Anzeigeoptionen (docs/05 §2.1) und ihre Anwendung auf eine Issue-Liste: Gruppierung, Sortierung,
 * Sub-Issues, erledigte Issues. Die Treffermenge bestimmt der Server; hier wird nur dargestellt.
 */
import type { Issue } from "@/gql/graphql";
import {
  isOpenType,
  priorityLabel,
  priorityRank,
  STATE_TYPE_ORDER,
  asStateType,
} from "@/features/issues/model";

export type Grouping = "none" | "status" | "assignee" | "project" | "priority" | "team" | "cycle";
export type Ordering = "priority" | "title" | "updatedAt" | "createdAt" | "dueDate" | "manual";
export type CompletedWindow = "all" | "pastDay" | "pastWeek" | "pastMonth" | "none";

export interface DisplayOptions {
  layout: "list" | "board";
  grouping: Grouping;
  ordering: Ordering;
  orderDirection: "asc" | "desc";
  showSubIssues: boolean;
  showCompletedIssues: CompletedWindow;
  showEmptyGroups: boolean;
}

export const DEFAULT_DISPLAY: DisplayOptions = {
  layout: "list",
  grouping: "status",
  ordering: "priority",
  orderDirection: "asc",
  showSubIssues: true,
  showCompletedIssues: "all",
  showEmptyGroups: false,
};

export const GROUPINGS: Grouping[] = [
  "none",
  "status",
  "assignee",
  "project",
  "priority",
  "team",
  "cycle",
];
export const ORDERINGS: Ordering[] = [
  "priority",
  "title",
  "updatedAt",
  "createdAt",
  "dueDate",
  "manual",
];
export const COMPLETED_WINDOWS: CompletedWindow[] = [
  "all",
  "pastDay",
  "pastWeek",
  "pastMonth",
  "none",
];

/** Wirksame Optionen: Defaults, überlagert von den (gültigen) gespeicherten Werten. */
export function mergeDisplay(saved: Record<string, unknown> | null | undefined): DisplayOptions {
  const out: DisplayOptions = { ...DEFAULT_DISPLAY };
  if (!saved) return out;
  const pick = <T extends string>(v: unknown, allowed: readonly T[]): T | undefined =>
    typeof v === "string" && (allowed as readonly string[]).includes(v) ? (v as T) : undefined;
  out.layout = pick(saved.layout, ["list", "board"] as const) ?? out.layout;
  out.grouping = pick(saved.grouping, GROUPINGS) ?? out.grouping;
  out.ordering = pick(saved.ordering, ORDERINGS) ?? out.ordering;
  out.orderDirection = pick(saved.orderDirection, ["asc", "desc"] as const) ?? out.orderDirection;
  out.showCompletedIssues =
    pick(saved.showCompletedIssues, COMPLETED_WINDOWS) ?? out.showCompletedIssues;
  if (typeof saved.showSubIssues === "boolean") out.showSubIssues = saved.showSubIssues;
  if (typeof saved.showEmptyGroups === "boolean") out.showEmptyGroups = saved.showEmptyGroups;
  return out;
}

const WINDOW_MS: Record<CompletedWindow, number> = {
  all: Infinity,
  none: 0,
  pastDay: 24 * 3600_000,
  pastWeek: 7 * 24 * 3600_000,
  pastMonth: 30 * 24 * 3600_000,
};

/** Blendet Sub-Issues und zu alte erledigte Issues aus; liefert auch die Zahl der ausgeblendeten (docs/05 §2). */
export function applyVisibility(
  issues: Issue[],
  o: DisplayOptions,
  now: number = Date.now(),
): { visible: Issue[]; hidden: number } {
  const visible = issues.filter((i) => {
    if (!o.showSubIssues && i.parentId) return false;
    if (!isOpenType(i.stateType) && o.showCompletedIssues !== "all") {
      const closed = Date.parse(i.completedAt ?? i.canceledAt ?? i.updatedAt);
      if (o.showCompletedIssues === "none") return false;
      return !Number.isNaN(closed) && now - closed <= WINDOW_MS[o.showCompletedIssues];
    }
    return true;
  });
  return { visible, hidden: issues.length - visible.length };
}

export function sortIssues(issues: Issue[], o: DisplayOptions): Issue[] {
  const dir = o.orderDirection === "desc" ? -1 : 1;
  const cmp = (a: Issue, b: Issue): number => {
    switch (o.ordering) {
      case "title":
        return a.title.localeCompare(b.title);
      case "updatedAt":
        return b.updatedAt.localeCompare(a.updatedAt);
      case "createdAt":
        return b.createdAt.localeCompare(a.createdAt);
      case "dueDate":
        return (a.dueDate ?? "9999").localeCompare(b.dueDate ?? "9999");
      case "manual":
        return a.sortOrder - b.sortOrder;
      default:
        return priorityRank(a.priority) - priorityRank(b.priority);
    }
  };
  return [...issues].sort(
    (a, b) =>
      dir * cmp(a, b) || a.identifier.localeCompare(b.identifier, undefined, { numeric: true }),
  );
}

export interface IssueGroup {
  key: string;
  label: string;
  issues: Issue[];
}

export interface GroupNames {
  assignee: (id: string | null) => string | null;
  project: (id: string | null) => string | null;
  team: (id: string) => string | null;
  cycle: (id: string) => string | null;
}

/** Gruppiert; leere Gruppen entfallen. Reihenfolge: Status nach Kategorie, Priorität nach Wichtigkeit, sonst Name. */
export function groupIssues(issues: Issue[], g: Grouping, names: GroupNames): IssueGroup[] {
  if (g === "none") return issues.length ? [{ key: "all", label: "All issues", issues }] : [];
  const keyOf = (i: Issue): [string, string] => {
    switch (g) {
      case "status":
        return [i.stateType + "/" + i.stateName, i.stateName];
      case "assignee":
        return [
          i.assigneeId ?? "none",
          i.assigneeId ? (names.assignee(i.assigneeId) ?? "Unknown") : "No assignee",
        ];
      case "project":
        return [
          i.projectId ?? "none",
          i.projectId ? (names.project(i.projectId) ?? "Project") : "No project",
        ];
      case "priority":
        return [String(i.priority), priorityLabel(i.priority)];
      case "team":
        return [i.teamId, names.team(i.teamId) ?? "Team"];
      case "cycle":
        return [i.cycleId ?? "none", i.cycleId ? (names.cycle(i.cycleId) ?? "Cycle") : "No cycle"];
    }
  };
  const map = new Map<string, IssueGroup>();
  for (const i of issues) {
    const [key, label] = keyOf(i);
    const grp = map.get(key) ?? { key, label, issues: [] };
    grp.issues.push(i);
    map.set(key, grp);
  }
  const groups = [...map.values()];
  if (g === "status") {
    groups.sort(
      (a, b) =>
        STATE_TYPE_ORDER.indexOf(asStateType(a.key.split("/")[0]!)) -
        STATE_TYPE_ORDER.indexOf(asStateType(b.key.split("/")[0]!)),
    );
  } else if (g === "priority") {
    groups.sort((a, b) => priorityRank(Number(a.key)) - priorityRank(Number(b.key)));
  } else {
    groups.sort((a, b) =>
      a.key === "none" ? 1 : b.key === "none" ? -1 : a.label.localeCompare(b.label),
    );
  }
  return groups;
}
