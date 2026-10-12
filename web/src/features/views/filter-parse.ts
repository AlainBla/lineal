/**
 * Umkehrung von `filter-model.ts`: gespeichertes Filter-JSON (`CustomView.filterData`) → Filter-Chips.
 * Was die Leiste nicht darstellen kann (verschachtelte or-Zweige, unbekannte Felder, ungewöhnliche
 * Operatoren), geht nicht verloren, sondern landet in `rest` und wird beim Anwenden und Speichern
 * unverändert mit UND angehängt („erweiterter Filter“, docs/05 §1.6).
 */
import {
  CLOSED_TYPES,
  chipToFilter,
  isDateField,
  newChipId,
  type Filter,
  type FilterChip,
  type FilterField,
  type FilterOp,
} from "./filter-model";

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const keysOf = (o: Obj) => Object.keys(o);
const strs = (v: unknown): string[] | null =>
  Array.isArray(v) && v.every((x) => typeof x === "string" || typeof x === "number")
    ? v.map(String)
    : null;
const sameSet = (a: string[], b: string[]) =>
  a.length === b.length && [...a].sort().join("|") === [...b].sort().join("|");

interface Parsed {
  op: FilterOp;
  values: string[];
  subTeams?: boolean;
}

/** `{ id: { in|nin|eq|neq } }` → Op + Werte. */
function idCmp(o: unknown): { neg: boolean; ids: string[] } | null {
  if (!isObj(o) || keysOf(o).length !== 1 || !isObj(o.id)) return null;
  const id = o.id;
  if (keysOf(id).length !== 1) return null;
  if (id.in !== undefined) {
    const v = strs(id.in);
    return v && { neg: false, ids: v };
  }
  if (id.nin !== undefined) {
    const v = strs(id.nin);
    return v && { neg: true, ids: v };
  }
  if (typeof id.eq === "string") return { neg: false, ids: [id.eq] };
  if (typeof id.neq === "string") return { neg: true, ids: [id.neq] };
  return null;
}

/**
 * Nullable-Relation (Assignee, Creator, Projekt, Zyklus …): `id`-Vergleich, `null`, optional `isMe`
 * und die zugehörigen `or` (is) / `and` (isNot) Verknüpfungen aus `filter-model.ts`.
 */
function relationParts(x: unknown, extras: (p: Obj) => { neg: boolean; value: string } | null) {
  const single = (p: unknown): { neg: boolean; values: string[] } | null => {
    if (!isObj(p)) return null;
    const c = idCmp(p);
    if (c) return { neg: c.neg, values: c.ids };
    if (keysOf(p).length === 1 && typeof p.null === "boolean") {
      return { neg: !p.null, values: ["none"] };
    }
    const e = extras(p);
    return e && { neg: e.neg, values: [e.value] };
  };
  if (!isObj(x)) return null;
  const direct = single(x);
  if (direct) return direct;
  const list = keysOf(x).length === 1 ? (x.or ?? x.and) : null;
  if (!Array.isArray(list) || list.length === 0) return null;
  const isOr = x.or !== undefined;
  const parts = list.map(single);
  if (parts.some((p) => p === null)) return null;
  const ps = parts as Array<{ neg: boolean; values: string[] }>;
  // `or` verknüpft positive, `and` negative Bedingungen.
  if (ps.some((p) => p.neg === isOr)) return null;
  return { neg: !isOr, values: ps.flatMap((p) => p.values) };
}

const userExtras = (p: Obj) => {
  const m = p.isMe;
  if (isObj(m) && typeof m.eq === "boolean" && keysOf(p).length === 1) {
    return { neg: !m.eq, value: "me" };
  }
  return null;
};

function parseUser(x: unknown): Parsed | null {
  const r = relationParts(x, userExtras);
  return r && { op: r.neg ? "isNot" : "is", values: r.values };
}

function parseRelation(x: unknown): Parsed | null {
  const r = relationParts(x, () => null);
  return r && { op: r.neg ? "isNot" : "is", values: r.values };
}

function parseCycle(x: unknown): Parsed | null {
  const flags: Record<string, string> = {
    isActive: "current",
    isNext: "upcoming",
    isPrevious: "previous",
  };
  const r = relationParts(x, (p) => {
    const k = keysOf(p)[0];
    const f = k ? p[k] : null;
    if (k && flags[k] && isObj(f) && f.eq === true && keysOf(p).length === 1) {
      return { neg: false, value: flags[k] };
    }
    return null;
  });
  return r && { op: r.neg ? "isNot" : "is", values: r.values };
}

function parseTeam(x: unknown): Parsed | null {
  const c = idCmp(x);
  if (c) return { op: c.neg ? "isNot" : "is", values: c.ids };
  if (isObj(x) && keysOf(x).length === 1 && Array.isArray(x.or) && x.or.length === 2) {
    const [a, b] = x.or as unknown[];
    const base = idCmp(a);
    const anc = isObj(b) && isObj(b.ancestors) && isObj(b.ancestors.some) ? b.ancestors.some : null;
    const ancCmp = anc ? idCmp(anc) : null;
    if (base && !base.neg && ancCmp && !ancCmp.neg && sameSet(base.ids, ancCmp.ids)) {
      return { op: "is", values: base.ids, subTeams: true };
    }
  }
  return null;
}

function parseLabels(x: unknown): Parsed | null {
  if (!isObj(x) || keysOf(x).length !== 1) return null;
  if (isObj(x.some)) {
    const c = idCmp(x.some);
    return c && !c.neg ? { op: "includeAny", values: c.ids } : null;
  }
  if (isObj(x.every)) {
    const c = idCmp(x.every);
    return c && c.neg ? { op: "excludeAny", values: c.ids } : null;
  }
  if (isObj(x.length) && keysOf(x.length).length === 1) {
    const l = x.length;
    if (l.eq === 0) return { op: "includeAny", values: ["none"] };
    if (l.gt === 0) return { op: "excludeAny", values: ["none"] };
  }
  return null;
}

function parseSubscribers(x: unknown): Parsed | null {
  const l = parseLabels(x);
  return l && l.values[0] !== "none" ? l : null;
}

function parseDate(x: unknown): Parsed | null {
  if (!isObj(x) || keysOf(x).length !== 1) return null;
  if (typeof x.lt === "string" && x.lt !== "P0D") return { op: "before", values: [x.lt] };
  if (typeof x.gt === "string") return { op: "after", values: [x.gt] };
  return null;
}

function parseEstimate(x: unknown): Parsed | null {
  if (!isObj(x) || keysOf(x).length !== 1) return null;
  const [k] = keysOf(x) as [string];
  const n = x[k];
  if (typeof n !== "number") return null;
  const op: Record<string, FilterOp> = {
    eq: "is",
    neq: "isNot",
    lt: "lessThan",
    gt: "greaterThan",
  };
  return op[k] ? { op: op[k], values: [String(n)] } : null;
}

function parseIn(x: unknown, numbers = false): Parsed | null {
  if (!isObj(x) || keysOf(x).length !== 1) return null;
  const [k] = keysOf(x) as [string];
  const list = k === "eq" || k === "neq" ? [x[k]] : x[k];
  const v = strs(list);
  if (!v || !["in", "nin", "eq", "neq"].includes(k)) return null;
  if (numbers && v.some((n) => Number.isNaN(Number(n)))) return null;
  return { op: k === "nin" || k === "neq" ? "isNot" : "is", values: v };
}

/** Ein einzelner Filterschlüssel `{ feld: bedingung }` → Chip, sonst `null`. */
function parseKey(key: string, x: unknown): Omit<FilterChip, "id"> | null {
  const done = (field: FilterField, p: Parsed | null) =>
    p && { field, op: p.op, values: p.values, ...(p.subTeams ? { subTeams: true } : {}) };
  switch (key) {
    case "assignee":
      return done("assignee", parseUser(x));
    case "creator":
      return done("creator", parseUser(x));
    case "state": {
      if (!isObj(x) || keysOf(x).length !== 1 || x.type === undefined) return null;
      return done("stateType", parseIn(x.type));
    }
    case "priority":
      return done("priority", parseIn(x, true));
    case "labels":
      return done("label", parseLabels(x));
    case "project":
      return done("project", parseRelation(x));
    case "cycle":
      return done("cycle", parseCycle(x));
    case "team":
      return done("team", parseTeam(x));
    case "subscribers":
      return done("subscribers", parseSubscribers(x));
    case "estimate":
      return done("estimate", parseEstimate(x));
    case "slaStatus":
      return done("slaStatus", parseIn(x));
    case "parent": {
      if (isObj(x) && keysOf(x).length === 1 && typeof x.null === "boolean") {
        return { field: "parent", op: x.null ? "is" : "isNot", values: ["none"] };
      }
      return null;
    }
    case "title":
      return isObj(x) && keysOf(x).length === 1 && typeof x.containsIgnoreCaseAndAccent === "string"
        ? { field: "title", op: "contains", values: [x.containsIgnoreCaseAndAccent] }
        : null;
    case "dueDate": {
      const d = parseDate(x);
      return d && { field: "dueDate", op: d.op, values: d.values };
    }
    default:
      if (["createdAt", "updatedAt", "startedAt", "completedAt", "canceledAt"].includes(key)) {
        const d = parseDate(x);
        return d && isDateField(key as FilterField)
          ? { field: key as FilterField, op: d.op, values: d.values }
          : null;
      }
      return null;
  }
}

/** `or`-Muster „Due date is not overdue“ aus `filter-model.ts`. */
function isNotOverdue(o: Obj): boolean {
  if (keysOf(o).length !== 1 || !Array.isArray(o.or) || o.or.length !== 2) return false;
  const [a, b] = o.or as unknown[];
  return (
    isObj(a) &&
    isObj(a.dueDate) &&
    a.dueDate.gte === "P0D" &&
    isObj(b) &&
    isObj(b.dueDate) &&
    b.dueDate.null === true
  );
}

/** `and` aus lauter Einzel-Labels → „include all of“. */
function parseIncludeAll(list: unknown[]): Omit<FilterChip, "id"> | null {
  const ids: string[] = [];
  for (const p of list) {
    if (!isObj(p) || keysOf(p).length !== 1 || !isObj(p.labels)) return null;
    const l = parseLabels(p.labels);
    if (!l || l.op !== "includeAny" || l.values.length !== 1 || l.values[0] === "none") return null;
    ids.push(l.values[0]!);
  }
  return ids.length >= 2 ? { field: "label", op: "includeAll", values: ids } : null;
}

const isOverdueDue = (p: unknown) =>
  isObj(p) && keysOf(p).length === 1 && isObj(p.dueDate) && p.dueDate.lt === "P0D";
const isClosedExclusion = (p: unknown) => {
  const c = isObj(p) && keysOf(p).length === 1 && isObj(p.state) ? p.state : null;
  const t = c && keysOf(c).length === 1 && isObj(c.type) ? c.type : null;
  const v = t ? strs(t.nin) : null;
  return !!v && sameSet(v, CLOSED_TYPES);
};

const OVERDUE = Symbol("overdue");

/** Zerlegt ein Filterobjekt in UND-Teile (`and` flach, mehrere Schlüssel einzeln). */
function conjuncts(f: unknown): unknown[] {
  if (f === null || f === undefined) return [];
  if (!isObj(f)) return [f];
  const keys = keysOf(f);
  if (keys.length === 0) return [];
  if (keys.length === 1 && keys[0] === "and" && Array.isArray(f.and)) {
    const asLabels = parseIncludeAll(f.and);
    return asLabels ? [f] : f.and.flatMap(conjuncts);
  }
  if (keys.length > 1) return keys.map((k) => ({ [k]: f[k] }));
  return [f];
}

export interface ParsedFilter {
  chips: FilterChip[];
  /** Nicht darstellbare Bedingungen („erweiterter Filter“), unverändert. */
  rest: Filter[];
}

/** Filter-JSON → Chips + Rest. `chipToFilter(chips) UND rest` ist semantisch das Eingabe-JSON. */
export function filterToChips(filter: unknown): ParsedFilter {
  const chips: FilterChip[] = [];
  const rest: Filter[] = [];
  let parts = conjuncts(filter);
  // „Overdue“ (Fälligkeit in der Vergangenheit UND nicht erledigt) liegt als zwei Teile vor.
  const od = parts.findIndex(isOverdueDue);
  const cl = parts.findIndex(isClosedExclusion);
  if (od >= 0 && cl >= 0) {
    parts = parts.map((p, i) => (i === od ? OVERDUE : p)).filter((_, i) => i !== cl);
  }
  for (const p of parts) {
    if (p === OVERDUE) {
      chips.push({ id: newChipId(), field: "dueDate", op: "is", values: ["overdue"] });
      continue;
    }
    if (!isObj(p)) {
      rest.push(p as Filter);
      continue;
    }
    const keys = keysOf(p);
    let chip: Omit<FilterChip, "id"> | null = null;
    if (isNotOverdue(p)) chip = { field: "dueDate", op: "isNot", values: ["overdue"] };
    else if (keys.length === 1 && keys[0] === "and" && Array.isArray(p.and)) {
      chip = parseIncludeAll(p.and);
    } else if (keys.length === 1 && keys[0] !== "or" && keys[0] !== "and") {
      chip = parseKey(keys[0]!, p[keys[0]!]);
    }
    // Sicherheitsnetz: nur Chips, die sich verlustfrei zurückübersetzen lassen.
    if (chip && sameShape(chipToFilter({ id: "x", ...chip }), p)) {
      chips.push({ id: newChipId(), ...chip });
    } else {
      rest.push(p as Filter);
    }
  }
  return { chips, rest };
}

/** Struktureller Vergleich, der `eq`/`neq`/Einzelwert-Schreibweisen als gleich ansieht. */
function sameShape(a: unknown, b: unknown): boolean {
  return canon(a) === canon(b);
}

function canon(v: unknown, key?: string): string {
  if (Array.isArray(v)) {
    const items = v.map((x) => canon(x));
    // `and`/`or` und Wertemengen (`in`/`nin`) sind reihenfolgeunabhängig.
    if (key === "and" || key === "or" || key === "in" || key === "nin") items.sort();
    return `[${items.join(",")}]`;
  }
  if (isObj(v)) {
    const entries = keysOf(v)
      .sort()
      .map((k) => {
        // `eq`/`neq` sind API-Komfort für einelementiges `in`/`nin` (docs/05 §1.5).
        const nk = k === "eq" ? "in" : k === "neq" ? "nin" : k;
        const val =
          (k === "eq" || k === "neq") && !isObj(v[k]) && !Array.isArray(v[k]) && v[k] !== null
            ? [v[k]]
            : v[k];
        return `${nk}:${canon(val, nk)}`;
      });
    return `{${entries.join(",")}}`;
  }
  return JSON.stringify(v);
}

/** Chips und Rest wieder zu einem Filter-JSON zusammensetzen (für Anwenden und Speichern). */
export function combineFilter(chipFilter: Filter, rest: Filter[]): Filter {
  const empty = Object.keys(chipFilter).length === 0;
  const parts = [...(empty ? [] : [chipFilter]), ...rest];
  if (parts.length === 0) return {};
  return parts.length === 1 ? parts[0]! : { and: parts };
}
