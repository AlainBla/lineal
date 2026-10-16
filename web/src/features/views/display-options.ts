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

export type Grouping =
  "none" | "status" | "assignee" | "project" | "priority" | "team" | "cycle" | "milestone";
export type Ordering =
  | "priority"
  | "title"
  | "status"
  | "assignee"
  | "estimate"
  | "updatedAt"
  | "createdAt"
  | "dueDate"
  | "linkCount"
  | "timeInStatus"
  | "manual";
export type CompletedWindow = "all" | "pastDay" | "pastWeek" | "pastMonth" | "none";
export type NestedSubIssues = "showAll" | "showMatching" | "hide";

/** Anzeige-Eigenschaften der Issue-Zeile (docs/05 §2.1); Schlüssel wie am Server. */
export const DISPLAY_PROPERTIES = [
  "id",
  "status",
  "assignee",
  "creator",
  "priority",
  "dueDate",
  "milestone",
  "cycle",
  "labels",
  "links",
  "timeInStatus",
  "created",
  "updated",
] as const;
export type DisplayProperty = (typeof DISPLAY_PROPERTIES)[number];
export const DEFAULT_PROPERTIES: DisplayProperty[] = [
  "id",
  "status",
  "assignee",
  "priority",
  "dueDate",
  "milestone",
  "labels",
  "updated",
];

export interface DisplayOptions {
  layout: "list" | "board";
  grouping: Grouping;
  /** Zweite Gruppierungsebene innerhalb jeder Gruppe (`none` = keine). */
  subGrouping: Grouping;
  ordering: Ordering;
  orderDirection: "asc" | "desc";
  showSubIssues: boolean;
  showCompletedIssues: CompletedWindow;
  showEmptyGroups: boolean;
  nestedSubIssues: NestedSubIssues;
  displayProperties: DisplayProperty[];
}

export const DEFAULT_DISPLAY: DisplayOptions = {
  layout: "list",
  grouping: "status",
  subGrouping: "none",
  ordering: "priority",
  orderDirection: "asc",
  showSubIssues: true,
  showCompletedIssues: "all",
  showEmptyGroups: false,
  nestedSubIssues: "hide",
  displayProperties: DEFAULT_PROPERTIES,
};

export const GROUPINGS: Grouping[] = [
  "none",
  "status",
  "assignee",
  "project",
  "priority",
  "team",
  "cycle",
  "milestone",
];
export const ORDERINGS: Ordering[] = [
  "priority",
  "title",
  "status",
  "assignee",
  "estimate",
  "updatedAt",
  "createdAt",
  "dueDate",
  "linkCount",
  "timeInStatus",
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
  out.subGrouping = pick(saved.subGrouping, GROUPINGS) ?? out.subGrouping;
  if (out.subGrouping === out.grouping) out.subGrouping = "none";
  out.nestedSubIssues =
    pick(saved.nestedSubIssues, ["showAll", "showMatching", "hide"] as const) ??
    out.nestedSubIssues;
  if (Array.isArray(saved.displayProperties)) {
    out.displayProperties = DISPLAY_PROPERTIES.filter((p) =>
      (saved.displayProperties as unknown[]).includes(p),
    );
  }
  if (typeof saved.showSubIssues === "boolean") out.showSubIssues = saved.showSubIssues;
  if (typeof saved.showEmptyGroups === "boolean") out.showEmptyGroups = saved.showEmptyGroups;
  return out;
}

/** Weicht die Anzeige vom Standard ab (Punkt am Display-Knopf)? Nur Schlüssel, die der Knopf steuert. */
export function isCustomDisplay(o: DisplayOptions): boolean {
  const d = DEFAULT_DISPLAY;
  return (
    o.layout !== d.layout ||
    o.grouping !== d.grouping ||
    o.subGrouping !== d.subGrouping ||
    o.ordering !== d.ordering ||
    o.orderDirection !== d.orderDirection ||
    o.showSubIssues !== d.showSubIssues ||
    o.showCompletedIssues !== d.showCompletedIssues ||
    o.showEmptyGroups !== d.showEmptyGroups ||
    o.nestedSubIssues !== d.nestedSubIssues ||
    o.displayProperties.join() !== d.displayProperties.join()
  );
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

/** Natürliche, groß-/kleinschreibungsunabhängige Titelsortierung (docs/05 §2.5). */
const compareTitles = (a: string, b: string): number =>
  a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });

/**
 * Sortierung nach docs/05 §2.5: Richtung wirkt nur auf vorhandene Werte, leere Werte (No priority,
 * keine Fälligkeit) stehen in beiden Richtungen zuletzt; `manual` ist nicht umkehrbar. Zeitpunkte
 * sind ihrer Richtung gemäß sortiert (createdAt asc = ältestes zuerst). Tie-Breaker: createdAt
 * absteigend, dann id.
 */
export interface SortContext {
  /** Anzeigename des Assignees für die Sortierung nach Assignee. */
  assignee?: (id: string | null) => string | null;
  /** Position des Status im Workflow (innerhalb der Kategorie). */
  statePosition?: (stateId: string) => number | undefined;
}

export function sortIssues(issues: Issue[], o: DisplayOptions, ctx: SortContext = {}): Issue[] {
  const dir = o.orderDirection === "desc" ? -1 : 1;
  /** Leere Werte stehen in beiden Richtungen zuletzt. */
  const emptyLast = <T>(
    a: T | null | undefined,
    b: T | null | undefined,
    cmpv: (x: T, y: T) => number,
  ) => {
    if (a == null || b == null) return (a == null ? 1 : 0) - (b == null ? 1 : 0);
    return dir * cmpv(a, b);
  };
  const cmp = (a: Issue, b: Issue): number => {
    switch (o.ordering) {
      case "title":
        return dir * compareTitles(a.title, b.title);
      case "updatedAt":
        return dir * a.updatedAt.localeCompare(b.updatedAt);
      case "createdAt":
        return dir * a.createdAt.localeCompare(b.createdAt);
      case "dueDate": {
        if (!a.dueDate || !b.dueDate) return (a.dueDate ? 0 : 1) - (b.dueDate ? 0 : 1);
        return dir * a.dueDate.localeCompare(b.dueDate);
      }
      case "status": {
        const ta = STATE_TYPE_ORDER.indexOf(asStateType(a.stateType));
        const tb = STATE_TYPE_ORDER.indexOf(asStateType(b.stateType));
        return (
          dir *
          (ta - tb ||
            (ctx.statePosition?.(a.stateId) ?? 0) - (ctx.statePosition?.(b.stateId) ?? 0) ||
            a.stateName.localeCompare(b.stateName))
        );
      }
      case "assignee": {
        const name = (i: Issue) =>
          i.assigneeId ? (ctx.assignee?.(i.assigneeId) ?? i.assigneeId) : null;
        return emptyLast(name(a), name(b), compareTitles);
      }
      case "estimate":
        return emptyLast(a.estimate, b.estimate, (x, y) => x - y);
      case "linkCount":
        return dir * ((a.linkCount ?? 0) - (b.linkCount ?? 0));
      case "timeInStatus":
        // `stateEnteredAt` aufsteigend = am längsten im Status zuerst (docs/05 §2.5).
        return dir * a.stateEnteredAt.localeCompare(b.stateEnteredAt);
      case "manual":
        return a.sortOrder - b.sortOrder;
      default: {
        if ((a.priority === 0) !== (b.priority === 0)) return a.priority === 0 ? 1 : -1;
        return dir * (priorityRank(a.priority) - priorityRank(b.priority));
      }
    }
  };
  return [...issues].sort(
    (a, b) => cmp(a, b) || b.createdAt.localeCompare(a.createdAt) || a.id.localeCompare(b.id),
  );
}

/** Voreingestellte Richtung je Sortierung (docs/05 §2.5): Zeitpunkte absteigend, sonst aufsteigend. */
export const defaultDirection = (o: Ordering): DisplayOptions["orderDirection"] =>
  o === "createdAt" || o === "updatedAt" || o === "linkCount" ? "desc" : "asc";

export interface IssueGroup {
  key: string;
  label: string;
  issues: Issue[];
  /** Zweite Gruppierungsebene (nur wenn `subGrouping` gesetzt ist). */
  subGroups?: IssueGroup[];
}

export interface GroupNames {
  assignee: (id: string | null) => string | null;
  project: (id: string | null) => string | null;
  team: (id: string) => string | null;
  cycle: (id: string) => string | null;
  /** Meilenstein-Name (F.07); fehlt, wo keine Projektdaten vorliegen. */
  milestone?: (id: string) => string | null;
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
      case "milestone":
        return [
          i.projectMilestoneId ?? "none",
          i.projectMilestoneId
            ? (names.milestone?.(i.projectMilestoneId) ?? "Milestone")
            : "No milestone",
        ];
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

/** Gruppen, die auch leer erscheinen sollen (`showEmptyGroups`); schon vorhandene bleiben unberührt. */
export function withEmptyGroups(
  groups: IssueGroup[],
  all: Array<{ key: string; label: string }>,
  grouping: Grouping,
): IssueGroup[] {
  if (all.length === 0) return groups;
  const have = new Set(groups.map((g) => g.key));
  const extra = all.filter((g) => !have.has(g.key)).map((g) => ({ ...g, issues: [] as Issue[] }));
  if (extra.length === 0) return groups;
  const order = new Map(all.map((g, i) => [g.key, i] as const));
  const merged = [...groups, ...extra];
  // Status und Priorität behalten ihre Reihenfolge, Meilensteine/Namen folgen der Quellliste.
  if (grouping === "milestone" || grouping === "cycle" || grouping === "team") {
    merged.sort((a, b) => (order.get(a.key) ?? 1e6) - (order.get(b.key) ?? 1e6));
  } else if (grouping === "priority") {
    merged.sort((a, b) => priorityRank(Number(a.key)) - priorityRank(Number(b.key)));
  }
  return merged;
}

/** Zweite Gruppierungsebene: jede Gruppe wird nach `sub` unterteilt. */
export function applySubGrouping(
  groups: IssueGroup[],
  sub: Grouping,
  names: GroupNames,
): IssueGroup[] {
  if (sub === "none") return groups;
  return groups.map((g) => ({ ...g, subGroups: groupIssues(g.issues, sub, names) }));
}

/** Alle Issues in Anzeigereihenfolge (für Tastaturnavigation), auch unter Untergruppen. */
export const flatIssues = (groups: IssueGroup[]): Issue[] =>
  groups.flatMap((g) => (g.subGroups ? g.subGroups.flatMap((s) => s.issues) : g.issues));

/**
 * Sub-Issues unter ihrem Parent einrücken (Nested sub-issues ≠ hide): Kinder, deren Parent in
 * derselben Liste steht, folgen direkt auf ihn (Tiefe 1+); alle anderen bleiben oben.
 */
export function nestIssues(issues: Issue[]): Array<{ issue: Issue; depth: number }> {
  const ids = new Set(issues.map((i) => i.id));
  const children = new Map<string, Issue[]>();
  for (const i of issues) {
    if (i.parentId && ids.has(i.parentId)) {
      children.set(i.parentId, [...(children.get(i.parentId) ?? []), i]);
    }
  }
  const out: Array<{ issue: Issue; depth: number }> = [];
  const walk = (i: Issue, depth: number, seen: Set<string>) => {
    if (seen.has(i.id)) return;
    seen.add(i.id);
    out.push({ issue: i, depth });
    for (const c of children.get(i.id) ?? []) walk(c, depth + 1, seen);
  };
  const seen = new Set<string>();
  for (const i of issues) if (!i.parentId || !ids.has(i.parentId)) walk(i, 0, seen);
  return out;
}
