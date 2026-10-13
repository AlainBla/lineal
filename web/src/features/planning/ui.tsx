import { Link } from "@tanstack/react-router";
import { useEffect, type ReactNode } from "react";

import { cn } from "@/lib/cn";

import { HEALTH_LABEL, type Health, useFiscalMonth } from "./queries";
import { asResolution, formatSpanDate } from "./roadmap-model";

const HEALTH_CLASS: Record<string, string> = {
  onTrack: "bg-emerald-500/15 text-emerald-500",
  atRisk: "bg-amber-500/15 text-amber-500",
  offTrack: "bg-red-500/15 text-red-500",
};

export function HealthBadge({ health }: { health: string | null }) {
  if (!health) return <span className="text-xs text-subtle-foreground">No update</span>;
  return (
    <span
      className={cn(
        "inline-flex rounded-full px-1.5 text-xs font-medium",
        HEALTH_CLASS[health] ?? "bg-surface-active text-muted-foreground",
      )}
    >
      {HEALTH_LABEL[health as Health] ?? health}
    </span>
  );
}

export function ProgressBar({ value, label }: { value: number; label?: string }) {
  const pct = Math.max(0, Math.min(100, Math.round(value * 100)));
  return (
    <div className="flex items-center gap-2">
      <div
        role="progressbar"
        aria-label={label ?? "Progress"}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        className="h-1.5 w-20 overflow-hidden rounded-full bg-surface-active"
      >
        <div className="h-full bg-primary" style={{ width: `${pct}%` }} />
      </div>
      <span className="text-xs text-muted-foreground tabular-nums">{pct}%</span>
    </div>
  );
}

/** Kopfzeile mit Titel, optionalen Tabs (Links) und Aktionen rechts. */
export function PageHeader({
  title,
  tabs,
  actions,
  docTitle,
}: {
  title: ReactNode;
  tabs?: Array<{ label: string; to: string; active: boolean }>;
  actions?: ReactNode;
  docTitle?: string;
}) {
  useEffect(() => {
    if (!docTitle) return;
    const prev = document.title;
    document.title = `${docTitle} · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [docTitle]);
  return (
    <header className="flex h-10 shrink-0 items-center gap-3 border-b border-border px-3">
      <h1 className="truncate text-sm font-medium">{title}</h1>
      {tabs && (
        <nav aria-label="Sections" className="flex items-center gap-1">
          {tabs.map((t) => (
            <Link
              key={t.label}
              to={t.to as "/"}
              aria-current={t.active ? "page" : undefined}
              className={cn(
                "rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                t.active && "bg-surface-active text-foreground",
              )}
            >
              {t.label}
            </Link>
          ))}
        </nav>
      )}
      <div className="ml-auto flex items-center gap-1">{actions}</div>
    </header>
  );
}

/**
 * Kalenderdatum (`YYYY-MM-DD` bzw. Zeitstempel mit Datumsteil) ohne Zeitzonenversatz: das Datum
 * wird als UTC gelesen und als UTC formatiert (B5.12). Mit Auflösung: „Q4 2026“ usw.
 */
export const shortDate = (iso: string | null | undefined, res?: string | null, fiscal = 0) => {
  if (!iso) return "–";
  const resolution = asResolution(res);
  if (resolution) return formatSpanDate(iso, resolution, fiscal);
  return new Date(`${iso.slice(0, 10)}T00:00:00Z`).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
};

/** Kalenderdatum mit Auflösung; liest das Fiskaljahr des Workspaces. */
export function DateText({
  iso,
  resolution,
}: {
  iso: string | null | undefined;
  resolution?: string | null;
}) {
  const fiscal = useFiscalMonth();
  return <>{shortDate(iso, resolution, fiscal)}</>;
}

/** Kalendertag eines Zeitpunkts in der Zeitzone `tz` (z. B. Team-Zeitzone). */
export const dayInZone = (iso: string, tz: string, offsetDays = 0) => {
  const t = new Date(new Date(iso).getTime() + offsetDays * 86_400_000);
  return t.toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: tz });
};
