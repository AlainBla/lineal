import { Inbox } from "lucide-react";

import { cn } from "@/lib/cn";

import { asStateType } from "./model";

interface StatusIconProps {
  /** Statuskategorie (`started`, `completed` …); bestimmt die Form. */
  type: string;
  /** Statusfarbe (`#RRGGBB`); ohne Angabe die Textfarbe. */
  color?: string | null;
  className?: string;
  title?: string;
}

const SAFE_COLOR = /^#[0-9A-Fa-f]{6}$/;
const safeColor = (c?: string | null) => (c && SAFE_COLOR.test(c) ? c : "currentColor");

/** Status-Icon, dessen Form die Kategorie kodiert (docs/08 §2.2). */
export function StatusIcon({ type, color, className, title }: StatusIconProps) {
  const t = asStateType(type);
  const c = safeColor(color);
  const common = {
    width: 14,
    height: 14,
    viewBox: "0 0 14 14",
    className: cn("shrink-0", className),
    role: "img" as const,
    "aria-label": title ?? t,
    "data-state-type": t,
  };
  if (t === "triage") {
    return <Inbox {...{ ...common, viewBox: undefined }} style={{ color: c }} strokeWidth={2} />;
  }
  return (
    <svg {...common} fill="none">
      {t === "backlog" && (
        <circle cx="7" cy="7" r="5.5" stroke={c} strokeWidth="1.5" strokeDasharray="1.8 1.8" />
      )}
      {t === "unstarted" && <circle cx="7" cy="7" r="5.5" stroke={c} strokeWidth="1.5" />}
      {t === "started" && (
        <>
          <circle cx="7" cy="7" r="5.5" stroke={c} strokeWidth="1.5" />
          <path d="M7 3.5a3.5 3.5 0 0 1 0 7z" fill={c} />
        </>
      )}
      {t === "completed" && (
        <>
          <circle cx="7" cy="7" r="6.25" fill={c} />
          <path
            d="m4.4 7.2 1.7 1.7 3.5-3.6"
            stroke="var(--background)"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </>
      )}
      {(t === "canceled" || t === "duplicate") && (
        <>
          <circle cx="7" cy="7" r="6.25" fill={c} />
          <path
            d="m4.9 4.9 4.2 4.2m0-4.2-4.2 4.2"
            stroke="var(--background)"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </>
      )}
    </svg>
  );
}

/** Prioritäts-Icon: Balken (Low–High), Urgent als gefülltes Quadrat mit Ausrufezeichen, sonst gestrichelt. */
export function PriorityIcon({ priority, className }: { priority: number; className?: string }) {
  const label = ["No priority", "Urgent", "High", "Medium", "Low"][priority] ?? "No priority";
  const props = {
    width: 14,
    height: 14,
    viewBox: "0 0 14 14",
    className: cn("shrink-0", className),
    role: "img" as const,
    "aria-label": `Priority: ${label}`,
    "data-priority": priority,
  };
  if (priority === 1) {
    return (
      <svg {...props} fill="none">
        <rect x="1.5" y="1.5" width="11" height="11" rx="2.5" fill="var(--destructive)" />
        <path d="M7 4v3.6M7 9.6v.2" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
    );
  }
  if (priority === 0) {
    return (
      <svg {...props} fill="none" className={cn("text-subtle-foreground", props.className)}>
        <path
          d="M2.5 7h2M6 7h2M9.5 7h2"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  // 2 = High (3 Balken), 3 = Medium (2), 4 = Low (1)
  const active = priority === 2 ? 3 : priority === 3 ? 2 : 1;
  return (
    <svg {...props} fill="none">
      {[0, 1, 2].map((i) => (
        <rect
          key={i}
          x={2 + i * 4}
          y={8 - i * 2.5}
          width="2.4"
          height={4 + i * 2.5}
          rx="0.6"
          fill="currentColor"
          className={i < active ? "text-foreground" : "text-subtle-foreground/40"}
        />
      ))}
    </svg>
  );
}

/** Kleiner Avatar mit Initialen. */
export function Avatar({ name, className }: { name: string; size?: number; className?: string }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]!.toUpperCase())
    .join("");
  return (
    <span
      aria-hidden
      className={cn(
        "inline-flex size-[18px] shrink-0 items-center justify-center rounded-full bg-surface-active text-[9px] font-medium text-muted-foreground",
        className,
      )}
    >
      {initials || "?"}
    </span>
  );
}

export function LabelDot({ color, className }: { color: string; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ backgroundColor: safeColor(color) }}
    />
  );
}
