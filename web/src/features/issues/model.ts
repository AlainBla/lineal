/**
 * Reine Logik der Issue-Ansichten (ohne React): Statuskategorien, Prioritäten, Ansichten
 * Active/Backlog/All, Board-Spalten, Drag-&-Drop-Statuswechsel, Schätzskalen. Fachregeln: docs/03, docs/05 §3.1.
 */
import type { Issue, IssueLabel, IssueRelations, RelatedIssue, WorkflowState } from "@/gql/graphql";

export type StateType =
  "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled" | "duplicate";

/** Reihenfolge der Kategorien im Workflow (docs/03 §1). */
export const STATE_TYPE_ORDER: StateType[] = [
  "triage",
  "backlog",
  "unstarted",
  "started",
  "completed",
  "canceled",
  "duplicate",
];

export const STATE_TYPE_LABEL: Record<StateType, string> = {
  triage: "Triage",
  backlog: "Backlog",
  unstarted: "Unstarted",
  started: "Started",
  completed: "Completed",
  canceled: "Canceled",
  duplicate: "Duplicate",
};

export function asStateType(v: string): StateType {
  return (STATE_TYPE_ORDER as string[]).includes(v) ? (v as StateType) : "backlog";
}

export const isOpenType = (t: string) =>
  t === "triage" || t === "backlog" || t === "unstarted" || t === "started";

/** Priority 0 = keine, 1 = Urgent … 4 = Low (Schema: `Issue.priority`). */
export const PRIORITIES = [
  { value: 0, label: "No priority" },
  { value: 1, label: "Urgent" },
  { value: 2, label: "High" },
  { value: 3, label: "Medium" },
  { value: 4, label: "Low" },
] as const;

export const priorityLabel = (p: number) =>
  PRIORITIES.find((x) => x.value === p)?.label ?? "No priority";

/** Wichtigkeit: Urgent zuerst, „No priority“ zuletzt. */
export const priorityRank = (p: number) => (p === 0 ? 5 : p);

export type TeamView = "active" | "backlog" | "all";

export function isTeamView(v: string): v is TeamView {
  return v === "active" || v === "backlog" || v === "all";
}

/** Zugehörigkeit eines Status-Typs zur Team-Ansicht (docs/05 §3.1); Triage ist immer ausgeblendet. */
export function stateTypeInView(type: string, view: TeamView): boolean {
  if (type === "triage") return false;
  if (view === "active") return type === "unstarted" || type === "started";
  if (view === "backlog") return type === "backlog";
  return true;
}

export function filterByView(issues: Issue[], view: TeamView): Issue[] {
  return issues.filter((i) => !i.trashed && stateTypeInView(i.stateType, view));
}

/** Status eines Teams in Workflow-Reihenfolge (Kategorie, dann `position`). */
export function sortStates(states: WorkflowState[]): WorkflowState[] {
  return [...states].sort((a, b) => {
    const ta = STATE_TYPE_ORDER.indexOf(asStateType(a.type));
    const tb = STATE_TYPE_ORDER.indexOf(asStateType(b.type));
    return ta - tb || a.position - b.position || a.name.localeCompare(b.name);
  });
}

/** Manuelle Sortierung: kleinerer `sortOrder` steht weiter oben (docs/03 §12). */
export const bySortOrder = (a: Issue, b: Issue) =>
  a.sortOrder - b.sortOrder ||
  a.identifier.localeCompare(b.identifier, undefined, { numeric: true });

export interface BoardColumn {
  state: WorkflowState;
  issues: Issue[];
}

export const COLUMN_PREFIX = "column:";

/**
 * Board-Spalten: eine je Status der Ansicht, Issues nach `sortOrder`. Leere Spalten bleiben
 * erhalten (der Aufrufer entscheidet über „Hidden columns“).
 */
export function buildBoardColumns(
  states: WorkflowState[],
  issues: Issue[],
  view: TeamView,
): BoardColumn[] {
  const visible = sortStates(states).filter((s) => stateTypeInView(s.type, view));
  const byState = new Map<string, Issue[]>();
  for (const i of filterByView(issues, view)) {
    const list = byState.get(i.stateId) ?? [];
    list.push(i);
    byState.set(i.stateId, list);
  }
  return visible.map((state) => ({
    state,
    issues: (byState.get(state.id) ?? []).sort(bySortOrder),
  }));
}

/** Listen-Gruppen nach Status (gleiche Struktur wie Board, leere Gruppen entfallen). */
export function groupByState(
  states: WorkflowState[],
  issues: Issue[],
  view: TeamView,
): BoardColumn[] {
  return buildBoardColumns(states, issues, view).filter((c) => c.issues.length > 0);
}

/** Spiegel von `rules::sort_order::between`. */
export function between(a: number | undefined, b: number | undefined): number {
  if (a !== undefined && b !== undefined) return (a + b) / 2;
  if (b !== undefined) return b - 1;
  if (a !== undefined) return a + 1;
  return 0;
}

export interface DropResult {
  issueId: string;
  /** `null`, wenn sich nichts ändert. */
  patch: { stateId?: string; sortOrder: number } | null;
}

/**
 * Drag & Drop auf dem Board. `overId` ist entweder eine Spalte (`column:<stateId>`) oder eine Karte
 * (Issue-ID). Wird auf eine Karte gelegt, landet das Issue davor (gleiche Spalte und Ziel weiter
 * unten: dahinter). Der neue `sortOrder` liegt zwischen den Nachbarn (docs/03 §12).
 */
export function applyDrop(
  columns: BoardColumn[],
  activeId: string,
  overId: string | null,
): DropResult {
  const none: DropResult = { issueId: activeId, patch: null };
  if (!overId || activeId === overId) return none;
  const from = columns.find((c) => c.issues.some((i) => i.id === activeId));
  if (!from) return none;

  let target: BoardColumn | undefined;
  let index: number;
  if (overId.startsWith(COLUMN_PREFIX)) {
    target = columns.find((c) => c.state.id === overId.slice(COLUMN_PREFIX.length));
    if (!target) return none;
    index = target.issues.filter((i) => i.id !== activeId).length;
  } else {
    target = columns.find((c) => c.issues.some((i) => i.id === overId));
    if (!target) return none;
    const rest = target.issues.filter((i) => i.id !== activeId);
    const overIdx = rest.findIndex((i) => i.id === overId);
    const oldIdx = target.issues.findIndex((i) => i.id === activeId);
    const overOldIdx = target.issues.findIndex((i) => i.id === overId);
    index = target === from && oldIdx < overOldIdx ? overIdx + 1 : overIdx;
  }

  const rest = target.issues.filter((i) => i.id !== activeId);
  const sortOrder = between(rest[index - 1]?.sortOrder, rest[index]?.sortOrder);
  const changedState = target.state.id !== from.state.id;
  if (!changedState && from.issues.findIndex((i) => i.id === activeId) === index) return none;
  return {
    issueId: activeId,
    patch: { ...(changedState ? { stateId: target.state.id } : {}), sortOrder },
  };
}

/** Fortschritt am Parent: direkte Kinder ohne canceled/duplicate (docs/03 §3.5). */
export function subIssueProgress(children: Issue[]): { done: number; total: number } {
  const counted = children.filter((c) => c.stateType !== "canceled" && c.stateType !== "duplicate");
  return { done: counted.filter((c) => c.stateType === "completed").length, total: counted.length };
}

const TSHIRT = [
  ["XS", 1],
  ["S", 2],
  ["M", 3],
  ["L", 5],
  ["XL", 8],
] as const;

/** Basis-Schätzskala eines Teams (docs/03 §2.5; erweiterte Werte hängen an Team-Optionen, die das Schema nicht liefert). */
export function estimateOptions(type: string): Array<{ value: number; label: string }> {
  switch (type) {
    case "exponential":
      return [1, 2, 4, 8, 16].map((v) => ({ value: v, label: String(v) }));
    case "fibonacci":
      return [1, 2, 3, 5, 8].map((v) => ({ value: v, label: String(v) }));
    case "linear":
      return [1, 2, 3, 4, 5].map((v) => ({ value: v, label: String(v) }));
    case "tShirt":
      return TSHIRT.map(([label, value]) => ({ value, label }));
    default:
      return [];
  }
}

export function estimateLabel(type: string, value: number | null): string | null {
  if (value === null) return null;
  return estimateOptions(type).find((o) => o.value === value)?.label ?? String(value);
}

/** Wichtigkeitssortierung (My Issues): Priorität, dann zuletzt geändert. */
export function byImportance(a: Issue, b: Issue): number {
  return (
    priorityRank(a.priority) - priorityRank(b.priority) || b.updatedAt.localeCompare(a.updatedAt)
  );
}

/** Kurze relative Zeit („3h“, „17d“); `now` injizierbar für Tests. */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d`;
  const mo = Math.floor(d / 30);
  return mo < 12 ? `${mo}mo` : `${Math.floor(mo / 12)}y`;
}

/** Felder, die optimistisch am Cache-Eintrag eines Issues geändert werden. */
export type IssuePatch = Partial<
  Pick<
    Issue,
    | "title"
    | "description"
    | "stateId"
    | "stateName"
    | "stateType"
    | "assigneeId"
    | "priority"
    | "estimate"
    | "labelIds"
    | "projectId"
    | "cycleId"
    | "dueDate"
    | "parentId"
    | "sortOrder"
  >
>;

/** Patch für Statuswechsel (Name und Kategorie folgen dem Status). */
export function describeStatePatch(
  states: WorkflowState[],
  stateId: string,
): Pick<IssuePatch, "stateId" | "stateName" | "stateType"> {
  const s = states.find((x) => x.id === stateId);
  return s ? { stateId, stateName: s.name, stateType: s.type } : { stateId };
}

/**
 * Label an-/abwählen. Ein Label einer `singleSelect`-Gruppe ersetzt gesetzte Geschwister
 * (docs/03 §5.5, wie der Server bei `addLabels`).
 */
export function toggleLabel(selected: string[], labelId: string, labels: IssueLabel[]): string[] {
  if (selected.includes(labelId)) return selected.filter((l) => l !== labelId);
  const label = labels.find((l) => l.id === labelId);
  const group = label?.parentId ? labels.find((l) => l.id === label.parentId) : undefined;
  if (group?.groupType === "singleSelect") {
    const siblings = new Set(labels.filter((l) => l.parentId === group.id).map((l) => l.id));
    return [...selected.filter((l) => !siblings.has(l)), labelId];
  }
  return [...selected, labelId];
}

export interface MovePreview {
  /** Verluste beim Teamwechsel (Client-Vorschau nach docs/02 §7.2, soweit die Daten reichen). */
  losses: string[];
}

/** Vorschau eines Teamwechsels: was geht verloren? Der Server entscheidet endgültig. */
export function previewMove(
  issue: Issue,
  target: {
    labels: IssueLabel[];
    projects: Array<{ id: string }>;
    estimationType: string;
    sourceEstimationType: string;
  },
): MovePreview {
  const losses: string[] = [];
  const available = new Set(target.labels.map((l) => l.id));
  const lostLabels = issue.labelIds.filter((id) => !available.has(id));
  if (lostLabels.length > 0) {
    losses.push(`${lostLabels.length} label${lostLabels.length === 1 ? "" : "s"} will be removed`);
  }
  if (issue.projectId && !target.projects.some((p) => p.id === issue.projectId)) {
    losses.push("Project will be removed");
  }
  if (issue.cycleId) losses.push("Cycle will be removed");
  if (issue.estimate !== null && target.estimationType !== target.sourceEstimationType) {
    losses.push("Estimate will be removed");
  }
  return { losses };
}

export type FocusDir = "up" | "down" | "left" | "right";

/**
 * Nächster Fokus in einem Raster aus Spalten (Liste = eine Spalte). Ohne Fokus gilt das erste
 * Element; links/rechts springen in die Nachbarspalte auf möglichst gleiche Höhe, leere werden übersprungen.
 */
export function nextFocus(
  columns: string[][],
  current: string | null,
  dir: FocusDir,
): string | null {
  const nonEmpty = columns.filter((c) => c.length > 0);
  if (nonEmpty.length === 0) return null;
  const col = columns.findIndex((c) => current !== null && c.includes(current));
  if (col < 0) return nonEmpty[0]![0]!;
  const row = columns[col]!.indexOf(current!);
  if (dir === "down") return columns[col]![Math.min(row + 1, columns[col]!.length - 1)]!;
  if (dir === "up") return columns[col]![Math.max(row - 1, 0)]!;
  const step = dir === "right" ? 1 : -1;
  for (let c = col + step; c >= 0 && c < columns.length; c += step) {
    const target = columns[c]!;
    if (target.length > 0) return target[Math.min(row, target.length - 1)]!;
  }
  return current;
}

export interface TypeGroup {
  type: StateType;
  label: string;
  issues: Issue[];
}

/** Gruppen nach Statuskategorie (teamübergreifend, z. B. My Issues); leere entfallen, Issues nach Wichtigkeit. */
export function groupByStateType(issues: Issue[]): TypeGroup[] {
  const order: StateType[] = [
    "started",
    "unstarted",
    "backlog",
    "triage",
    "completed",
    "canceled",
    "duplicate",
  ];
  return order
    .map((type) => ({
      type,
      label: STATE_TYPE_LABEL[type],
      issues: issues.filter((i) => asStateType(i.stateType) === type).sort(byImportance),
    }))
    .filter((g) => g.issues.length > 0);
}

/** My Issues „Assigned“: offene Issues plus innerhalb des letzten Tages abgeschlossene (docs/05 §3.2 `pastDay`). */
export function filterAssigned(issues: Issue[], now: number = Date.now()): Issue[] {
  const dayAgo = now - 24 * 3600 * 1000;
  return issues.filter((i) => {
    if (i.trashed) return false;
    if (isOpenType(i.stateType)) return true;
    const closedAt = Date.parse(i.completedAt ?? i.canceledAt ?? i.updatedAt);
    return !Number.isNaN(closedAt) && closedAt >= dayAgo;
  });
}

export type RelationChoice = "blocked-by" | "blocking" | "related" | "duplicate";

export const RELATION_CHOICES: Array<{ id: RelationChoice; label: string; keys: string }> = [
  { id: "blocked-by", label: "Blocked by", keys: "m b" },
  { id: "blocking", label: "Blocking", keys: "m x" },
  { id: "related", label: "Related to", keys: "m r" },
  { id: "duplicate", label: "Duplicate of", keys: "m m" },
];

/**
 * Übersetzt die Sicht des aktuellen Issues in `issueRelationCreate` (docs/03 §4.1): „A blocks B“ ist
 * eine Zeile; „blocked by“ dreht die Richtung um.
 */
export function relationInput(
  choice: RelationChoice,
  thisId: string,
  otherId: string,
): { issueId: string; relatedIssueId: string; type: "blocks" | "related" | "duplicate" } {
  switch (choice) {
    case "blocked-by":
      return { issueId: otherId, relatedIssueId: thisId, type: "blocks" };
    case "blocking":
      return { issueId: thisId, relatedIssueId: otherId, type: "blocks" };
    case "related":
      return { issueId: thisId, relatedIssueId: otherId, type: "related" };
    case "duplicate":
      return { issueId: thisId, relatedIssueId: otherId, type: "duplicate" };
  }
}

/** URL-Slug aus dem Titel (kosmetisch, docs/08 §1.3). */
export function slugify(title: string): string {
  const s = title
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/ß/g, "ss")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60)
    .replace(/-+$/g, "");
  return s || "issue";
}

/** Gruppen der Relationsanzeige in Reihenfolge und Beschriftung (Sicht des aktuellen Issues). */
export function relationGroups(r: IssueRelations): Array<{ label: string; items: RelatedIssue[] }> {
  return [
    { label: "Blocked by", items: r.blockedBy },
    { label: "Blocking", items: r.blocks },
    { label: "Related", items: r.relatedTo },
    { label: "Duplicate of", items: r.duplicateOf ? [r.duplicateOf] : [] },
    { label: "Duplicates", items: r.duplicates },
  ].filter((g) => g.items.length > 0);
}

/** Pfad zum Issue-Detail (docs/08 §1.3). */
export const issuePath = (ws: string, identifier: string) => `/${ws}/issue/${identifier}`;
