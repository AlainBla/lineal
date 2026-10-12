import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { Diamond } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { errorText } from "@/features/issues/error-text";
import { EmptyState, ErrorState, ListSkeleton } from "@/features/issues/states";
import { cn } from "@/lib/cn";

import { initiativeItems, projectItems } from "./roadmap-items";
import {
  barBox,
  buildRuler,
  dayX,
  dragSpan,
  formatSpanDate,
  fromDay,
  GROUP_LABEL,
  groupItems,
  itemRange,
  PX_PER_DAY,
  todayIso,
  ZOOM_LABEL,
  ZOOMS,
  type DragMode,
  type GroupBy,
  type RoadmapItem,
  type Span,
  type Zoom,
} from "./roadmap-model";
import {
  useInitiative,
  useInitiatives,
  useProjects,
  useUpdateInitiative,
  useUpdateProject,
} from "./queries";
import { PageHeader } from "./ui";

const LABEL_W = 224;
const ROW_H = 32;

export type RoadmapScope =
  { kind: "workspace" } | { kind: "team"; teamKey: string } | { kind: "initiative"; ref: string };

interface Drag {
  id: string;
  mode: DragMode;
  startX: number;
  dx: number;
}

/** Eingabe für `projectUpdate`/`initiativeUpdate` aus der geänderten Spanne (nur geänderte Felder). */
export function spanInput(item: RoadmapItem, next: Span): Record<string, unknown> {
  const input: Record<string, unknown> = {};
  const { span } = item;
  if (item.startEditable && next.startDate !== span.startDate) {
    input.startDate = next.startDate;
    if (next.startRes) input.startDateResolution = next.startRes;
  }
  if (next.targetDate !== span.targetDate) {
    input.targetDate = next.targetDate;
    if (next.targetRes) input.targetDateResolution = next.targetRes;
  }
  return input;
}

function describe(span: Span) {
  return `${formatSpanDate(span.startDate, span.startRes)} – ${formatSpanDate(span.targetDate, span.targetRes)}`;
}

function Bar({
  item,
  span,
  left,
  width,
  dx,
  px,
  onStartDrag,
  onKey,
}: {
  item: RoadmapItem;
  span: Span;
  left: number;
  width: number;
  dx: Drag | null;
  px: number;
  onStartDrag: (item: RoadmapItem, e: React.PointerEvent, mode: DragMode) => void;
  onKey: (item: RoadmapItem, e: React.KeyboardEvent) => void;
}) {
  const active = dx?.id === item.id ? dx : null;
  let l = left;
  let w = width;
  if (active) {
    const eff: DragMode = !item.startEditable ? "end" : active.mode;
    if (eff === "move") l += active.dx;
    else if (eff === "start") {
      l += active.dx;
      w -= active.dx;
    } else w += active.dx;
    w = Math.max(w, px);
  }
  const bg = item.color ?? "var(--color-primary)";
  return (
    <div
      role="button"
      tabIndex={0}
      data-roadmap-bar={item.id}
      aria-label={`${item.name}, ${describe(span)}. Drag to change the dates, arrow keys move them.`}
      title={`${item.name} · ${describe(span)}`}
      onPointerDown={(e) => onStartDrag(item, e, "move")}
      onKeyDown={(e) => onKey(item, e)}
      className={cn(
        "absolute top-1.5 flex h-5 touch-none items-center overflow-hidden rounded text-xs text-white select-none",
        active ? "cursor-grabbing opacity-80 ring-2 ring-ring" : "cursor-grab",
      )}
      style={{ left: l, width: w, background: bg }}
    >
      {item.startEditable && (
        <span
          data-handle="start"
          aria-hidden
          className="absolute inset-y-0 left-0 w-1.5 cursor-ew-resize bg-black/20"
          onPointerDown={(e) => {
            e.stopPropagation();
            onStartDrag(item, e, "start");
          }}
        />
      )}
      <span className="truncate px-2">{item.name}</span>
      <span
        data-handle="end"
        aria-hidden
        className="absolute inset-y-0 right-0 w-1.5 cursor-ew-resize bg-black/20"
        onPointerDown={(e) => {
          e.stopPropagation();
          onStartDrag(item, e, "end");
        }}
      />
    </div>
  );
}

/**
 * Zeitachsen-Ansicht (docs/04 §2.5): Balken von Start- bis Zieldatum, Meilensteine als Marker,
 * „Heute“-Linie, Zoom Woche bis Jahr, Gruppierung. Ziehen verschiebt bzw. verändert die Daten über
 * `projectUpdate`/`initiativeUpdate`; die Rechteprüfung liegt beim Server, ein Fehler stellt die
 * alte Position wieder her und wird angezeigt.
 */
export function RoadmapView({
  items,
  kind,
  today = todayIso(),
}: {
  items: RoadmapItem[];
  kind: "project" | "initiative";
  today?: string;
}) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const updateProject = useUpdateProject();
  const updateInitiative = useUpdateInitiative();
  const [zoom, setZoom] = useState<Zoom>("month");
  const [groupBy, setGroupBy] = useState<GroupBy>("none");
  const [drag, setDrag] = useState<Drag | null>(null);
  const [override, setOverride] = useState<Record<string, Span>>({});
  const [error, setError] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const dragRef = useRef<Drag | null>(null);
  const px = PX_PER_DAY[zoom];

  const shown = useMemo(
    () => items.map((i) => (override[i.id] ? { ...i, span: override[i.id]! } : i)),
    [items, override],
  );
  const ruler = useMemo(() => buildRuler(shown, zoom, today), [shown, zoom, today]);
  const scheduled = shown.filter((i) => itemRange(i));
  const unscheduled = shown.filter((i) => !itemRange(i));
  const groups = useMemo(() => groupItems(scheduled, groupBy), [scheduled, groupBy]);
  const width = ruler.days * px;

  const commit = useCallback(
    (item: RoadmapItem, next: Span) => {
      const input = spanInput(item, next);
      if (Object.keys(input).length === 0) return;
      setError(null);
      setOverride((o) => ({ ...o, [item.id]: next }));
      const mutation = item.kind === "project" ? updateProject : updateInitiative;
      mutation
        .mutateAsync({ id: item.id, input })
        .catch((e: unknown) => setError(errorText(e)))
        .finally(() =>
          qc.refetchQueries({ queryKey: ["planning"] }).finally(() =>
            setOverride((o) => {
              const rest = { ...o };
              delete rest[item.id];
              return rest;
            }),
          ),
        );
    },
    [qc, updateProject, updateInitiative],
  );

  const finish = (item: RoadmapItem, d: Drag) => {
    if (Math.abs(d.dx) < 4) {
      void navigate({ to: item.href as "/" });
      return;
    }
    const eff: DragMode = !item.startEditable ? "end" : d.mode;
    const next = dragSpan(item.span, eff, Math.round(d.dx / px));
    if (next) commit(item, next);
  };

  const startDrag = (item: RoadmapItem, e: React.PointerEvent, mode: DragMode) => {
    if (e.button !== 0) return;
    const d: Drag = { id: item.id, mode, startX: e.clientX, dx: 0 };
    dragRef.current = d;
    setDrag(d);
    const move = (ev: PointerEvent) => {
      const cur = dragRef.current;
      if (!cur) return;
      const next = { ...cur, dx: ev.clientX - cur.startX };
      dragRef.current = next;
      setDrag(next);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      const cur = dragRef.current;
      dragRef.current = null;
      setDrag(null);
      if (cur) finish(item, cur);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const onKey = (item: RoadmapItem, e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      void navigate({ to: item.href as "/" });
      return;
    }
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    // Eine Auflösung wandert in ganzen Perioden: ein Tag Rundung reicht für „eine Periode“ nicht.
    const unit = item.span.targetRes || item.span.startRes ? 31 : 1;
    const delta = e.key === "ArrowLeft" ? -unit : unit;
    const next = dragSpan(item.span, e.shiftKey || !item.startEditable ? "end" : "move", delta);
    if (next) commit(item, next);
  };

  const scrollToToday = () => {
    const el = scroller.current;
    if (el) el.scrollLeft = Math.max(0, dayX(today, ruler, zoom) - el.clientWidth / 3);
  };

  const todayX = dayX(today, ruler, zoom);
  const rows: React.ReactNode[] = [];
  for (const g of groups) {
    if (g.label) {
      rows.push(
        <div
          key={`h-${g.key}`}
          role="presentation"
          className="flex h-7 items-center border-b border-border bg-surface px-3 text-xs font-medium text-muted-foreground"
          style={{ position: "sticky", left: 0, width: LABEL_W + width }}
        >
          {g.label}
          <span className="ml-2 tabular-nums">{g.items.length}</span>
        </div>,
      );
    }
    for (const it of g.items) {
      const box = barBox(it, ruler, zoom)!;
      rows.push(
        <div
          key={`${g.key}-${it.id}`}
          role="row"
          className="flex border-b border-border hover:bg-surface-hover"
          style={{ height: ROW_H, width: LABEL_W + width }}
        >
          <div
            role="rowheader"
            className="sticky left-0 z-10 flex shrink-0 items-center gap-2 truncate border-r border-border bg-background px-3 text-sm"
            style={{ width: LABEL_W }}
          >
            <span
              aria-hidden
              className="size-2.5 shrink-0 rounded-sm"
              style={{ background: it.color ?? "var(--color-primary)" }}
            />
            <span className="truncate">{it.name}</span>
          </div>
          <div role="gridcell" className="relative" style={{ width }}>
            <Bar
              item={it}
              span={it.span}
              left={box.left}
              width={box.width}
              dx={drag}
              px={px}
              onStartDrag={startDrag}
              onKey={onKey}
            />
            {it.milestones.map((m) => (
              <span
                key={m.id}
                data-milestone={m.id}
                title={`${m.name} · ${formatSpanDate(m.targetDate, null)}`}
                className="pointer-events-auto absolute top-2 -ml-1.5 text-amber-500"
                style={{ left: dayX(m.targetDate, ruler, zoom) }}
              >
                <Diamond className="size-3 fill-current" aria-label={`Milestone ${m.name}`} />
              </span>
            ))}
          </div>
        </div>,
      );
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1">
        <div role="radiogroup" aria-label="Zoom" className="flex gap-0.5">
          {ZOOMS.map((z) => (
            <Button
              key={z}
              type="button"
              variant="ghost"
              size="sm"
              role="radio"
              aria-checked={zoom === z}
              className={zoom === z ? "bg-surface-active" : undefined}
              onClick={() => setZoom(z)}
            >
              {ZOOM_LABEL[z]}
            </Button>
          ))}
        </div>
        <label className="flex items-center gap-1 text-xs text-muted-foreground">
          Group by
          <select
            aria-label="Group by"
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value as GroupBy)}
            className="h-7 rounded-md border border-input bg-transparent px-1 text-sm text-foreground"
          >
            {(Object.keys(GROUP_LABEL) as GroupBy[])
              .filter((g) => kind === "project" || g !== "initiative")
              .map((g) => (
                <option key={g} value={g}>
                  {g === "initiative" && kind === "initiative"
                    ? "Parent initiative"
                    : GROUP_LABEL[g]}
                </option>
              ))}
          </select>
        </label>
        <Button type="button" variant="ghost" size="sm" onClick={scrollToToday}>
          Today
        </Button>
        {error && (
          <span role="alert" className="text-xs text-destructive">
            {error}
          </span>
        )}
      </div>
      {scheduled.length === 0 ? (
        <EmptyState
          title="Nothing scheduled"
          description="Set a start and target date to see it on the roadmap."
        />
      ) : (
        <div
          ref={scroller}
          role="grid"
          aria-label="Roadmap"
          className="relative min-h-0 flex-1 overflow-auto"
        >
          <div className="relative" style={{ width: LABEL_W + width }}>
            <div
              role="row"
              className="sticky top-0 z-20 flex h-8 border-b border-border bg-background"
            >
              <div
                className="sticky left-0 z-30 shrink-0 border-r border-border bg-background"
                style={{ width: LABEL_W }}
              />
              <div className="relative" style={{ width }}>
                {ruler.ticks.map((t) => (
                  <span
                    key={t.day}
                    data-tick={fromDay(t.day)}
                    className="absolute inset-y-0 flex items-center border-l border-border px-1 text-xs text-muted-foreground"
                    style={{ left: t.x, width: t.width }}
                  >
                    <span className="truncate">{t.label}</span>
                  </span>
                ))}
              </div>
            </div>
            {rows}
            <div
              aria-hidden
              data-today={today}
              className="pointer-events-none absolute top-0 bottom-0 z-10 w-px bg-red-500"
              style={{ left: LABEL_W + todayX }}
            />
          </div>
        </div>
      )}
      {unscheduled.length > 0 && (
        <section aria-label="Without dates" className="border-t border-border px-3 py-2">
          <h2 className="mb-1 text-xs font-medium text-muted-foreground">
            Without dates ({unscheduled.length})
          </h2>
          <ul className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
            {unscheduled.map((i) => (
              <li key={i.id}>
                <a
                  href={i.href}
                  className="hover:underline"
                  onClick={(e) => {
                    e.preventDefault();
                    void navigate({ to: i.href as "/" });
                  }}
                >
                  {i.name}
                </a>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** Roadmap eines Bereichs: Workspace (Projekte/Initiativen), Team oder Initiative. */
export function RoadmapPage({
  ws,
  scope,
  embedded = false,
}: {
  ws: string;
  scope: RoadmapScope;
  /** Eingebettet (Tab der Initiative): ohne eigene Kopfzeile. */
  embedded?: boolean;
}) {
  const [kind, setKind] = useState<"project" | "initiative">("project");
  const teamKey = scope.kind === "team" ? scope.teamKey : undefined;
  const projects = useProjects({ team: teamKey, includeSubTeams: teamKey ? true : undefined });
  const initiatives = useInitiatives();
  const parent = useInitiative(
    scope.kind === "initiative" ? scope.ref : "",
    scope.kind === "initiative",
  );
  const effKind = scope.kind === "workspace" ? kind : "project";

  const items = useMemo(() => {
    if (effKind === "initiative") return initiativeItems(initiatives.data ?? [], ws);
    let list = projects.data ?? [];
    if (scope.kind === "initiative") {
      const id = parent.data?.id;
      list = id ? list.filter((p) => p.initiatives.some((i) => i.id === id)) : [];
    }
    return projectItems(list, ws);
  }, [effKind, initiatives.data, projects.data, parent.data, scope.kind, ws]);

  const q = effKind === "initiative" ? initiatives : projects;
  let body;
  if (q.isLoading || (scope.kind === "initiative" && parent.isLoading)) body = <ListSkeleton />;
  else if (q.isError)
    body = (
      <ErrorState
        error={q.error}
        title="Could not load the roadmap"
        onRetry={() => void q.refetch()}
      />
    );
  else body = <RoadmapView items={items} kind={effKind} />;

  const header = (
    <>
      {scope.kind === "workspace" && (
        <div role="radiogroup" aria-label="Show" className="flex gap-0.5">
          {(["project", "initiative"] as const).map((k) => (
            <Button
              key={k}
              type="button"
              variant="ghost"
              size="sm"
              role="radio"
              aria-checked={kind === k}
              className={kind === k ? "bg-surface-active" : undefined}
              onClick={() => setKind(k)}
            >
              {k === "project" ? "Projects" : "Initiatives"}
            </Button>
          ))}
        </div>
      )}
    </>
  );

  if (embedded) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {scope.kind === "workspace" && <div className="flex px-3 py-1">{header}</div>}
        {body}
      </div>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Roadmap"
        docTitle={teamKey ? `${teamKey} · Roadmap` : "Roadmap"}
        actions={header}
      />
      {body}
    </div>
  );
}
