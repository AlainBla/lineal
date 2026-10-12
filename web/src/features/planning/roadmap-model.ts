/**
 * Rechenkern der Roadmap (docs/04 §2.5): Kalendertage, Zoomstufen, Balkengeometrie, Verschieben mit
 * Auflösung (Monat/Quartal/Halbjahr/Jahr) und Gruppierung. Rein und ohne React/DOM, damit alles
 * ohne Browser testbar ist. Alle Daten sind `YYYY-MM-DD` und werden in UTC gerechnet.
 */

export type Resolution = "month" | "quarter" | "halfYear" | "year";
export type Zoom = "week" | "month" | "quarter" | "year";
export const ZOOMS: Zoom[] = ["week", "month", "quarter", "year"];
export const ZOOM_LABEL: Record<Zoom, string> = {
  week: "Week",
  month: "Month",
  quarter: "Quarter",
  year: "Year",
};
/** Breite eines Tages in Pixeln je Zoomstufe. */
export const PX_PER_DAY: Record<Zoom, number> = { week: 24, month: 7, quarter: 2.4, year: 0.8 };

export type GroupBy = "none" | "initiative" | "team" | "status";
export const GROUP_LABEL: Record<GroupBy, string> = {
  none: "No grouping",
  initiative: "Initiative",
  team: "Team",
  status: "Status",
};

const DAY = 86_400_000;

export const toDay = (iso: string): number => {
  const [y, m, d] = iso.slice(0, 10).split("-").map(Number);
  return Date.UTC(y!, (m ?? 1) - 1, d ?? 1) / DAY;
};
export const fromDay = (n: number): string => new Date(n * DAY).toISOString().slice(0, 10);
export const todayIso = (now = new Date()): string =>
  new Date(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate())).toISOString().slice(0, 10);

function ymd(day: number): { y: number; m: number; d: number } {
  const dt = new Date(day * DAY);
  return { y: dt.getUTCFullYear(), m: dt.getUTCMonth(), d: dt.getUTCDate() };
}
const utcDay = (y: number, m: number, d: number) => Date.UTC(y, m, d) / DAY;

/** Monate je Auflösung. */
const MONTHS: Record<Resolution, number> = { month: 1, quarter: 3, halfYear: 6, year: 12 };

/** Erster Tag der Periode, die `day` enthält. */
function periodStart(day: number, res: Resolution): number {
  const { y, m } = ymd(day);
  const n = MONTHS[res];
  return utcDay(y, m - (m % n), 1);
}
/** Letzter Tag der Periode, die `day` enthält. */
function periodEnd(day: number, res: Resolution): number {
  const { y, m } = ymd(day);
  const n = MONTHS[res];
  return utcDay(y, m - (m % n) + n, 1) - 1;
}

/** Normalisiert ein Datum auf die Auflösung (Start = Periodenbeginn, Ziel = Periodenende, docs/04 §2.5). */
export function normalizeDate(iso: string, res: Resolution | null, role: "start" | "target") {
  if (!res) return iso.slice(0, 10);
  const d = toDay(iso);
  return fromDay(role === "start" ? periodStart(d, res) : periodEnd(d, res));
}

/** Verschiebt ein Datum um `delta` Tage bzw., bei Auflösung, um ganze Perioden. */
function shiftDate(iso: string, res: Resolution | null, role: "start" | "target", delta: number) {
  const day = toDay(iso);
  if (!res) return fromDay(day + delta);
  const n = MONTHS[res];
  // 30,44 Tage je Monat; Rundung auf ganze Perioden (mindestens eine Periode bei klarer Bewegung).
  const periods = Math.round(delta / (30.44 * n));
  if (periods === 0) return normalizeDate(iso, res, role);
  const { y, m } = ymd(periodStart(day, res));
  const first = utcDay(y, m + periods * n, 1);
  return fromDay(role === "start" ? first : periodEnd(first, res));
}

export interface Span {
  startDate: string | null;
  targetDate: string | null;
  startRes: Resolution | null;
  targetRes: Resolution | null;
}

export type DragMode = "move" | "start" | "end";

/**
 * Neue Daten nach Ziehen um `deltaDays`: `move` verschiebt beide Enden, `start`/`end` nur eines.
 * `null`, wenn sich nichts ändert oder das Ergebnis ungültig wäre (Start nach Ziel).
 */
export function dragSpan(span: Span, mode: DragMode, deltaDays: number): Span | null {
  const { startDate, targetDate, startRes, targetRes } = span;
  const next: Span = { ...span };
  if ((mode === "move" || mode === "start") && startDate) {
    next.startDate = shiftDate(startDate, startRes, "start", deltaDays);
  }
  if ((mode === "move" || mode === "end") && targetDate) {
    next.targetDate = shiftDate(targetDate, targetRes, "target", deltaDays);
  }
  if (next.startDate === startDate && next.targetDate === targetDate) return null;
  if (next.startDate && next.targetDate && toDay(next.startDate) > toDay(next.targetDate)) {
    return null;
  }
  return next;
}

export interface RoadmapMilestone {
  id: string;
  name: string;
  targetDate: string;
}

export interface RoadmapItem {
  id: string;
  kind: "project" | "initiative";
  name: string;
  href: string;
  color: string | null;
  status: string;
  team: string | null;
  initiatives: Array<{ id: string; name: string }>;
  span: Span;
  /** Initiativen haben nur ein Zieldatum; ihr Start wird aus `startedAt` abgeleitet und nicht verschoben. */
  startEditable: boolean;
  milestones: RoadmapMilestone[];
}

/** Sichtbarer Zeitraum eines Elements in Tagen (inklusive Zieltag); `null` ohne Termine. */
export function itemRange(item: RoadmapItem): { start: number; end: number } | null {
  const { startDate, targetDate } = item.span;
  if (!startDate && !targetDate) return null;
  const start = toDay(startDate ?? targetDate!);
  const end = toDay(targetDate ?? startDate!);
  return { start: Math.min(start, end), end: Math.max(start, end) + 1 };
}

export interface Ruler {
  /** Erster dargestellter Tag. */
  origin: number;
  /** Tage insgesamt. */
  days: number;
  ticks: Array<{ day: number; label: string; x: number; width: number }>;
}

const MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

function tickLabel(day: number, zoom: Zoom): string {
  const { y, m, d } = ymd(day);
  switch (zoom) {
    case "week":
      return `${d} ${MONTH_NAMES[m]}`;
    case "month":
      return `${MONTH_NAMES[m]} ${y}`;
    case "quarter":
      return `Q${Math.floor(m / 3) + 1} ${y}`;
    case "year":
      return String(y);
  }
}

/** Beginn der Zoom-Einheit, die `day` enthält (Woche: Montag). */
function unitStart(day: number, zoom: Zoom): number {
  if (zoom === "week") return day - ((((day + 3) % 7) + 7) % 7); // 1970-01-01 war ein Donnerstag
  return periodStart(day, zoom === "month" ? "month" : zoom === "quarter" ? "quarter" : "year");
}
function unitNext(day: number, zoom: Zoom): number {
  if (zoom === "week") return day + 7;
  return periodEnd(day, zoom === "month" ? "month" : zoom === "quarter" ? "quarter" : "year") + 1;
}

/** Zeitachse, die alle Elemente und „heute“ umfasst, mit je einer Einheit Rand. */
export function buildRuler(items: RoadmapItem[], zoom: Zoom, today: string): Ruler {
  const t = toDay(today);
  let lo = t;
  let hi = t;
  for (const it of items) {
    const r = itemRange(it);
    if (r) {
      lo = Math.min(lo, r.start);
      hi = Math.max(hi, r.end);
    }
    for (const ms of it.milestones) {
      lo = Math.min(lo, toDay(ms.targetDate));
      hi = Math.max(hi, toDay(ms.targetDate) + 1);
    }
  }
  const origin = unitStart(unitStart(lo, zoom) - 1, zoom);
  let end = unitNext(hi, zoom);
  end = unitNext(end, zoom);
  const ticks: Ruler["ticks"] = [];
  const px = PX_PER_DAY[zoom];
  for (let d = origin; d < end; d = unitNext(d, zoom)) {
    const next = unitNext(d, zoom);
    ticks.push({ day: d, label: tickLabel(d, zoom), x: (d - origin) * px, width: (next - d) * px });
  }
  return { origin, days: end - origin, ticks };
}

export interface Group {
  key: string;
  label: string;
  items: RoadmapItem[];
}

/** Gruppiert (Projekte mit mehreren Initiativen erscheinen in jeder; ohne Zuordnung am Ende). */
export function groupItems(items: RoadmapItem[], by: GroupBy): Group[] {
  if (by === "none") return [{ key: "all", label: "", items }];
  const map = new Map<string, Group>();
  const put = (key: string, label: string, it: RoadmapItem) => {
    const g = map.get(key) ?? { key, label, items: [] };
    g.items.push(it);
    map.set(key, g);
  };
  for (const it of items) {
    if (by === "initiative") {
      if (it.initiatives.length === 0) put("none", "No initiative", it);
      for (const i of it.initiatives) put(i.id, i.name, it);
    } else if (by === "team") {
      put(it.team ?? "none", it.team ?? "No team", it);
    } else {
      put(it.status, it.status, it);
    }
  }
  const groups = [...map.values()];
  const none = groups.findIndex((g) => g.key === "none");
  if (none >= 0) groups.push(...groups.splice(none, 1));
  return groups;
}

/** Position und Breite eines Balkens in Pixeln (mindestens ein Tag breit). */
export function barBox(item: RoadmapItem, ruler: Ruler, zoom: Zoom) {
  const r = itemRange(item);
  if (!r) return null;
  const px = PX_PER_DAY[zoom];
  return { left: (r.start - ruler.origin) * px, width: Math.max((r.end - r.start) * px, 6) };
}

export const dayX = (iso: string, ruler: Ruler, zoom: Zoom) =>
  (toDay(iso) - ruler.origin) * PX_PER_DAY[zoom];

/** Anzeige eines Datums mit Auflösung (docs/04 §2.5: „Q4 2026“, „H2 2026“). */
export function formatSpanDate(iso: string | null, res: Resolution | null): string {
  if (!iso) return "–";
  const { y, m, d } = ymd(toDay(iso));
  switch (res) {
    case "month":
      return `${MONTH_NAMES[m]} ${y}`;
    case "quarter":
      return `Q${Math.floor(m / 3) + 1} ${y}`;
    case "halfYear":
      return `H${m < 6 ? 1 : 2} ${y}`;
    case "year":
      return String(y);
    default:
      return `${MONTH_NAMES[m]} ${d}, ${y}`;
  }
}

export const asResolution = (v: string | null | undefined): Resolution | null =>
  v === "month" || v === "quarter" || v === "halfYear" || v === "year" ? v : null;
