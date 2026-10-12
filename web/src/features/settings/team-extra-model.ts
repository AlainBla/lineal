/** Reine Hilfen für SLA-Regeln und wiederkehrende Issues (WP30). */

export const PRIORITY_NAMES = ["No priority", "Urgent", "High", "Medium", "Low"] as const;

// ---------- SLA-Bedingungen ----------

export interface SlaConditions {
  priorities: number[];
  labelIds: string[];
}

/**
 * Baut den `IssueFilter` einer Regel aus den einfachen Bedingungen des Editors:
 * `priority` (eine oder mehrere Stufen) und `labels` (mindestens eines der Labels).
 */
export function conditionsToFilter(c: SlaConditions): Record<string, unknown> {
  const filter: Record<string, unknown> = {};
  const prios = [...new Set(c.priorities)].sort((a, b) => a - b);
  if (prios.length === 1) filter.priority = { eq: prios[0] };
  else if (prios.length > 1) filter.priority = { in: prios };
  if (c.labelIds.length > 0) filter.labels = { some: { id: { in: c.labelIds } } };
  return filter;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * Liest die einfachen Bedingungen zurück. `null`, wenn der Filter mehr enthält, als der Editor
 * abbilden kann (dann bleibt der Filter unverändert und wird nur angezeigt).
 */
export function filterToConditions(filter: unknown): SlaConditions | null {
  if (!isObj(filter)) return null;
  const out: SlaConditions = { priorities: [], labelIds: [] };
  for (const [key, value] of Object.entries(filter)) {
    if (key === "priority" && isObj(value)) {
      const keys = Object.keys(value);
      if (keys.length === 1 && keys[0] === "eq" && typeof value.eq === "number")
        out.priorities = [value.eq];
      else if (
        keys.length === 1 &&
        keys[0] === "in" &&
        Array.isArray(value.in) &&
        value.in.every((n) => typeof n === "number")
      )
        out.priorities = value.in as number[];
      else return null;
    } else if (key === "labels" && isObj(value) && isObj(value.some)) {
      const id = value.some.id;
      if (
        Object.keys(value).length === 1 &&
        Object.keys(value.some).length === 1 &&
        isObj(id) &&
        Object.keys(id).length === 1 &&
        Array.isArray(id.in) &&
        id.in.every((x) => typeof x === "string")
      )
        out.labelIds = id.in as string[];
      else return null;
    } else return null;
  }
  return out;
}

export function describeConditions(filter: unknown, labelName: (id: string) => string): string {
  const c = filterToConditions(filter);
  if (!c) return "Custom conditions";
  const parts: string[] = [];
  if (c.priorities.length)
    parts.push(`Priority ${c.priorities.map((p) => PRIORITY_NAMES[p] ?? String(p)).join(" or ")}`);
  if (c.labelIds.length) parts.push(`Label ${c.labelIds.map(labelName).join(" or ")}`);
  return parts.length ? parts.join(" and ") : "All issues";
}

// ---------- SLA-Dauer ----------

export type DurationUnit = "hours" | "days" | "weeks";

/** ISO-8601-Dauer (`PT36H`, `P3D`, `P2W`). */
export function durationToIso(amount: number, unit: DurationUnit): string {
  const n = Math.max(1, Math.floor(amount));
  return unit === "hours" ? `PT${n}H` : unit === "days" ? `P${n}D` : `P${n}W`;
}

export const DURATION_PRESETS: Array<{ iso: string; label: string }> = [
  { iso: "PT12H", label: "12 hours" },
  { iso: "PT24H", label: "24 hours" },
  { iso: "PT48H", label: "48 hours" },
  { iso: "P1W", label: "1 week" },
  { iso: "P2W", label: "2 weeks" },
  { iso: "P4W", label: "4 weeks" },
];

/** Liest `PT36H`, `P3D`, `P2W`; `null` bei anderen Formen. */
export function parseDuration(
  iso: string | null | undefined,
): { amount: number; unit: DurationUnit } | null {
  const m = /^P(?:(\d+)([DW])|T(\d+)H)$/.exec(iso ?? "");
  if (!m) return null;
  if (m[3]) return { amount: Number(m[3]), unit: "hours" };
  return { amount: Number(m[1]), unit: m[2] === "W" ? "weeks" : "days" };
}

export function describeDuration(iso: string | null | undefined, businessDays: boolean): string {
  const preset = DURATION_PRESETS.find((p) => p.iso === iso);
  const p = parseDuration(iso);
  if (!p) return iso ?? "–";
  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
  if (businessDays) {
    const days = p.unit === "weeks" ? p.amount * 5 : p.amount;
    return plural(days, "business day");
  }
  if (preset) return preset.label;
  return plural(p.amount, p.unit.slice(0, -1));
}

// ---------- Recurring ----------

export const WEEKDAYS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

/** `"08:30"` → Minuten seit Mitternacht; ungültig → `null`. */
export function timeToMinute(t: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(t);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

export function minuteToTime(minute: number): string {
  const h = Math.floor(minute / 60);
  const m = minute % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

export interface Rhythm {
  frequency: string;
  interval: number;
  weekday?: number | null;
  dayOfMonth?: number | null;
  minuteOfDay: number;
}

const ordinal = (n: number) => {
  const mod100 = n % 100;
  if (mod100 >= 11 && mod100 <= 13) return `${n}th`;
  return `${n}${["th", "st", "nd", "rd"][n % 10 > 3 ? 0 : n % 10] ?? "th"}`;
};

/** „Every week on Monday at 08:00“. */
export function describeRhythm(r: Rhythm): string {
  const at = minuteToTime(r.minuteOfDay);
  const every = (unit: string) =>
    r.interval === 1 ? `Every ${unit}` : `Every ${r.interval} ${unit}s`;
  if (r.frequency === "weekly")
    return `${every("week")} on ${WEEKDAYS[(r.weekday ?? 1) - 1] ?? "?"} at ${at}`;
  if (r.frequency === "monthly")
    return `${every("month")} on the ${ordinal(r.dayOfMonth ?? 1)} at ${at}`;
  return `${every("day")} at ${at}`;
}

/** Zeitpunkt in der Zeitzone des Teams; bei ungültiger Zone in lokaler Zeit. */
export function formatInZone(iso: string | null | undefined, timeZone: string | undefined): string {
  if (!iso) return "–";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "–";
  const opts: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };
  try {
    return new Intl.DateTimeFormat(undefined, { ...opts, timeZone }).format(d);
  } catch {
    return new Intl.DateTimeFormat(undefined, opts).format(d);
  }
}

// ---------- Relative Fristen im Template (`dueIn`, `targetIn`) ----------

/** `P3D`, `P2W`, `P1W3D` → Tage als Text; leer/ungültig → `""`. */
export function relativeDays(iso: unknown): string {
  if (typeof iso !== "string") return "";
  const m = /^P(?:(\d+)W)?(?:(\d+)D)?$/.exec(iso);
  if (!m || (!m[1] && !m[2])) return "";
  return String(Number(m[1] ?? 0) * 7 + Number(m[2] ?? 0));
}

/** Gültig = leer oder ganze Zahl 0–3650 (Server prüft die Obergrenze erneut). */
export function relativeDaysValid(days: string): boolean {
  if (days.trim() === "") return true;
  const n = Number(days);
  return Number.isInteger(n) && n >= 0 && n <= 3650;
}

/** Tage → `P{n}D`; leer → `undefined`. */
export function relativeDaysToIso(days: string): string | undefined {
  return days.trim() === "" ? undefined : `P${Number(days)}D`;
}
