/**
 * Filtermodell der Filter-Bar: Chips (UI-Operator + Werte) → Filter-JSON nach docs/05 §1.5–1.7.
 * Rein und ohne React, damit die Übersetzung getestet werden kann. Der Server validiert und normalisiert
 * (`filter::parse_issue_filter`); Chips werden mit UND verknüpft.
 */

export type FilterField =
  | "assignee"
  | "creator"
  | "stateType"
  | "priority"
  | "label"
  | "project"
  | "cycle"
  | "team"
  | "dueDate"
  | "createdAt"
  | "title";

export type FilterOp =
  "is" | "isNot" | "includeAny" | "includeAll" | "excludeAny" | "before" | "after" | "contains";

export interface FilterChip {
  id: string;
  field: FilterField;
  op: FilterOp;
  /** Werte: IDs bzw. Sonderwerte (`me`, `none`, `current`, `upcoming`, `previous`, `overdue`), Datum, Text. */
  values: string[];
  /** Nur `team`: Sub-Teams einschließen (`ancestors`, docs/05 §1.7). */
  subTeams?: boolean;
}

export type Filter = Record<string, unknown>;

export const FIELD_LABEL: Record<FilterField, string> = {
  assignee: "Assignee",
  creator: "Creator",
  stateType: "Status type",
  priority: "Priority",
  label: "Labels",
  project: "Project",
  cycle: "Cycle",
  team: "Team",
  dueDate: "Due date",
  createdAt: "Created",
  title: "Title",
};

/** Operatoren je Feld (docs/05 §1.5). */
export const FIELD_OPS: Record<FilterField, FilterOp[]> = {
  assignee: ["is", "isNot"],
  creator: ["is", "isNot"],
  stateType: ["is", "isNot"],
  priority: ["is", "isNot"],
  label: ["includeAny", "includeAll", "excludeAny"],
  project: ["is", "isNot"],
  cycle: ["is", "isNot"],
  team: ["is", "isNot"],
  dueDate: ["is", "isNot", "before", "after"],
  createdAt: ["before", "after"],
  title: ["contains"],
};

export const OP_LABEL: Record<FilterOp, string> = {
  is: "is",
  isNot: "is not",
  includeAny: "include any of",
  includeAll: "include all of",
  excludeAny: "exclude if any of",
  before: "before",
  after: "after",
  contains: "contains",
};

const CLOSED_TYPES = ["completed", "canceled", "duplicate"];

/** Assignee/Creator (docs/05 §1.6, §1.7). */
function userFilter(op: FilterOp, values: string[]): Filter {
  const ids = values.filter((v) => v !== "me" && v !== "none");
  const me = values.includes("me");
  const none = values.includes("none");
  if (op === "isNot") {
    const parts: Filter[] = [];
    if (ids.length) parts.push({ id: { nin: ids } });
    if (me) parts.push({ isMe: { eq: false } });
    if (none) parts.push({ null: false });
    return parts.length === 1 ? parts[0]! : { and: parts };
  }
  const parts: Filter[] = [];
  if (ids.length) parts.push({ id: { in: ids } });
  if (me) parts.push({ isMe: { eq: true } });
  if (none) parts.push({ null: true });
  return parts.length === 1 ? parts[0]! : { or: parts };
}

function labelFilter(chip: FilterChip): Filter {
  const { op, values } = chip;
  if (values.length === 1 && values[0] === "none") {
    return op === "excludeAny"
      ? { labels: { length: { gt: 0 } } }
      : { labels: { length: { eq: 0 } } };
  }
  const some = (ids: string[]) => ({ labels: { some: { id: { in: ids } } } });
  switch (op) {
    case "includeAll":
      return values.length === 1 ? some(values) : { and: values.map((v) => some([v])) };
    case "excludeAny":
      return { labels: { every: { id: { nin: values } } } };
    default:
      return some(values);
  }
}

function cycleFilter(chip: FilterChip): Filter {
  const ids = chip.values.filter((v) => !["current", "upcoming", "previous", "none"].includes(v));
  if (chip.op === "isNot") return { cycle: { id: { nin: ids } } };
  const parts: Filter[] = [];
  if (ids.length) parts.push({ id: { in: ids } });
  if (chip.values.includes("current")) parts.push({ isActive: { eq: true } });
  if (chip.values.includes("upcoming")) parts.push({ isNext: { eq: true } });
  if (chip.values.includes("previous")) parts.push({ isPrevious: { eq: true } });
  if (chip.values.includes("none")) parts.push({ null: true });
  return { cycle: parts.length === 1 ? parts[0]! : { or: parts } };
}

function relationFilter(key: string, chip: FilterChip): Filter {
  const ids = chip.values.filter((v) => v !== "none");
  const none = chip.values.includes("none");
  if (chip.op === "isNot") {
    const parts: Filter[] = [];
    if (ids.length) parts.push({ id: { nin: ids } });
    if (none) parts.push({ null: false });
    return { [key]: parts.length === 1 ? parts[0]! : { and: parts } };
  }
  const parts: Filter[] = [];
  if (ids.length) parts.push({ id: { in: ids } });
  if (none) parts.push({ null: true });
  return { [key]: parts.length === 1 ? parts[0]! : { or: parts } };
}

function teamFilter(chip: FilterChip): Filter {
  const ids = chip.values;
  if (chip.op === "isNot") return { team: { id: { nin: ids } } };
  if (!chip.subTeams) return { team: { id: { in: ids } } };
  return {
    team: { or: [{ id: { in: ids } }, { ancestors: { some: { id: { in: ids } } } }] },
  };
}

function dueDateFilter(chip: FilterChip): Filter {
  const v = chip.values[0] ?? "";
  if (chip.op === "before") return { dueDate: { lt: v } };
  if (chip.op === "after") return { dueDate: { gt: v } };
  // „Overdue“: erledigte Issues sind nicht überfällig (docs/05 §1.7).
  if (chip.op === "isNot") {
    return { or: [{ dueDate: { gte: "P0D" } }, { dueDate: { null: true } }] };
  }
  return { dueDate: { lt: "P0D" }, state: { type: { nin: CLOSED_TYPES } } };
}

/** Übersetzt einen Chip in sein Filter-JSON. */
export function chipToFilter(chip: FilterChip): Filter {
  const { field, op, values } = chip;
  switch (field) {
    case "assignee":
    case "creator":
      return { [field]: userFilter(op, values) };
    case "stateType":
      return { state: { type: { [op === "isNot" ? "nin" : "in"]: values } } };
    case "priority":
      return { priority: { [op === "isNot" ? "nin" : "in"]: values.map(Number) } };
    case "label":
      return labelFilter(chip);
    case "project":
      return relationFilter("project", chip);
    case "cycle":
      return cycleFilter(chip);
    case "team":
      return teamFilter(chip);
    case "dueDate":
      return dueDateFilter(chip);
    case "createdAt":
      return { createdAt: { [op === "before" ? "lt" : "gt"]: values[0] ?? "" } };
    case "title":
      return { title: { containsIgnoreCaseAndAccent: values[0] ?? "" } };
  }
}

/** Ein Chip ist auswertbar, wenn er Werte hat (leere Chips werden ignoriert). */
export const isChipComplete = (c: FilterChip) =>
  c.values.length > 0 && c.values.every((v) => v !== "");

/** Alle Chips → ein Filter (leer = `{}`, ein Chip = dessen Objekt, mehrere = `and`). */
export function buildFilter(chips: FilterChip[]): Filter {
  const parts = chips.filter(isChipComplete).map(chipToFilter);
  if (parts.length === 0) return {};
  return parts.length === 1 ? parts[0]! : { and: parts };
}

export const isEmptyFilter = (f: Filter) => Object.keys(f).length === 0;

/** Kurzbeschreibung eines Chips für die Anzeige; `name` löst IDs zu Namen auf. */
export function describeChip(
  chip: FilterChip,
  name: (field: FilterField, v: string) => string,
): string {
  const vals = chip.values.map((v) => name(chip.field, v)).join(", ");
  return `${FIELD_LABEL[chip.field]} ${OP_LABEL[chip.op]} ${vals}`;
}

let counter = 0;
export const newChipId = () => `chip-${++counter}`;
