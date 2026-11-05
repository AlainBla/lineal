import {
  closestCorners,
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
  pointerWithin,
  type CollisionDetection,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Link } from "@tanstack/react-router";
import { Ellipsis, Eye, EyeOff, Plus } from "lucide-react";
import { Fragment, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Issue } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useShortcuts } from "@/lib/shortcuts";

import type { DisplayProperty } from "@/features/views/display-options";

import type { BoardVisibility } from "./board-visibility";
import { StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { issueMenuHandlers } from "./context-menu-store";
import { IssueCardBody, IssueCardHover, SelectBox } from "./issue-row";
import {
  applyDrop,
  COLUMN_PREFIX,
  HIDDEN_PREFIX,
  issuePath,
  type BoardColumn,
  type DropResult,
  type HiddenColumn,
} from "./model";
import type { Selection } from "./selection";

interface BoardProps {
  columns: BoardColumn[];
  /** Sichtbare und versteckte Spalten (`useBoardVisibility`). */
  visibility: BoardVisibility;
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  /** Wird mit dem Ergebnis eines gültigen Drops aufgerufen (Statuswechsel und/oder neue Reihenfolge). */
  onDrop: (result: DropResult & { issue: Issue }) => void;
  onCreateInColumn?: (stateId: string) => void;
  selection?: Selection;
  /** Sichtbare Eigenschaften der Karten (Display properties). */
  display?: readonly DisplayProperty[];
  /**
   * Verschiebt der Drop die Karte sichtbar (Statuswechsel, bei manueller Sortierung auch
   * Umsortieren)? Dann entfällt die Rückflug-Animation; sonst fliegt die Karte an ihren Platz zurück.
   */
  movesCard?: (patch: NonNullable<DropResult["patch"]>) => boolean;
}

function Card({
  issue,
  ws,
  lookup,
  focused,
  onFocus,
  selection,
  display,
}: {
  issue: Issue;
  ws: string;
  lookup: IssueLookup;
  focused: boolean;
  onFocus: (id: string) => void;
  selection?: Selection;
  display?: readonly DisplayProperty[];
}) {
  const selected = selection?.has(issue.id) ?? false;
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: issue.id,
  });
  const menu = issueMenuHandlers(issue, selection);
  const link = (
    <Link
      ref={setNodeRef}
      to={issuePath(ws, issue.identifier) as "/"}
      data-issue-id={issue.id}
      data-focused={focused || undefined}
      {...attributes}
      role="link"
      {...listeners}
      data-selected={selected || undefined}
      onFocus={() => onFocus(issue.id)}
      onClick={(e) => {
        if (selection?.handleClick(issue.id, e)) e.preventDefault();
      }}
      onContextMenu={menu.onContextMenu}
      onKeyDown={(e) => {
        menu.onKeyDown(e);
        if (!e.defaultPrevented) listeners?.onKeyDown?.(e);
      }}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        "group relative block rounded-lg border border-border bg-card p-2.5 text-card-foreground outline-none hover:border-border-strong focus-visible:border-ring",
        focused && "border-border-strong",
        selected && "border-primary bg-surface-active",
        isDragging && "opacity-40",
      )}
    >
      {selection && (
        <span className="absolute right-2 bottom-2">
          <SelectBox issue={issue} selection={selection} />
        </span>
      )}
      <IssueCardBody issue={issue} lookup={lookup} display={display} />
    </Link>
  );
  // Während des Ziehens keine Hover-Details.
  return isDragging ? (
    link
  ) : (
    <IssueCardHover issue={issue} lookup={lookup}>
      {link}
    </IssueCardHover>
  );
}

function Column({
  column,
  ws,
  lookup,
  focusedId,
  onFocus,
  onCreate,
  selection,
  display,
  menu,
}: {
  column: BoardColumn;
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  onCreate?: (stateId: string) => void;
  selection?: Selection;
  display?: readonly DisplayProperty[];
  /** Eintrag im „…“-Menü des Spaltenkopfs. */
  menu?: { label: string; hidden: boolean; run: () => void };
}) {
  const { setNodeRef, isOver } = useDroppable({ id: COLUMN_PREFIX + column.state.id });
  const { state, issues } = column;
  return (
    <section
      aria-label={state.name}
      data-column={state.id}
      className="flex w-72 shrink-0 flex-col rounded-xl bg-surface"
    >
      {/* Bleibt beim Scrollen oben stehen (sticky im Scroll-Container des Boards, `-top-4` gleicht
          dessen Padding aus; alle Spalten sind gleich hoch, damit jeder Kopf stehen bleibt); der
          Farbbalken in Statusfarbe macht die Spalte auch weit unten eindeutig erkennbar. */}
      <header
        className="sticky -top-4 z-10 flex h-10 items-center gap-2 rounded-t-xl border-t-[3px] border-b border-b-border bg-surface px-3 text-sm shadow-[0_6px_8px_-6px_rgb(0_0_0/0.12)]"
        style={{ borderTopColor: state.color ?? undefined }}
        data-testid="column-header"
      >
        <StatusIcon type={state.type} color={state.color} />
        <span className="truncate font-semibold">{state.name}</span>
        <span className="rounded-full bg-background px-1.5 text-xs font-medium text-muted-foreground tabular-nums">
          {issues.length}
        </span>
        {menu && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="ml-auto size-6"
                aria-label={`${state.name} column options`}
              >
                <Ellipsis className="size-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuItem onSelect={menu.run}>
                {menu.hidden ? <Eye /> : <EyeOff />}
                {menu.label}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
        {onCreate && (
          <Button
            variant="ghost"
            size="icon"
            className={cn("size-6", !menu && "ml-auto")}
            aria-label={`Create issue in ${state.name}`}
            onClick={() => onCreate(state.id)}
          >
            <Plus className="size-3.5" />
          </Button>
        )}
      </header>
      <div
        ref={setNodeRef}
        className={cn(
          "flex min-h-24 flex-1 flex-col gap-2 rounded-b-xl p-2 transition-colors",
          isOver && "bg-surface-hover",
        )}
      >
        <SortableContext items={issues.map((i) => i.id)} strategy={verticalListSortingStrategy}>
          {issues.map((issue) => (
            <Card
              key={issue.id}
              issue={issue}
              ws={ws}
              lookup={lookup}
              focused={focusedId === issue.id}
              onFocus={onFocus}
              selection={selection}
              display={display}
            />
          ))}
        </SortableContext>
        {issues.length === 0 && (
          <p className="px-1 py-4 text-center text-xs text-subtle-foreground">No issues</p>
        )}
      </div>
    </section>
  );
}

/** Eintrag unter „Hidden columns“: Klick blendet die Spalte ein oder aus, Karten lassen sich ablegen. */
function HiddenColumnEntry({
  entry: { column, open },
  onToggle,
}: {
  entry: HiddenColumn;
  onToggle: (stateId: string) => void;
}) {
  const { state, issues } = column;
  // Eingeblendet ist die Spalte selbst das Ziel. Duplicate entsteht nur über die Relation.
  const { setNodeRef, isOver } = useDroppable({
    id: HIDDEN_PREFIX + state.id,
    disabled: open || state.type === "duplicate",
  });
  return (
    <div ref={setNodeRef}>
      <Button
        variant="ghost"
        size="sm"
        className={cn(
          "w-full justify-start gap-2",
          open && "bg-surface-active",
          isOver && "bg-surface-active ring-2 ring-primary",
        )}
        aria-pressed={open}
        aria-label={`${open ? "Hide" : "Show"} ${state.name} column`}
        onClick={() => onToggle(state.id)}
      >
        <StatusIcon type={state.type} color={state.color} />
        <span className="truncate">{state.name}</span>
        <span className="ml-auto text-xs text-muted-foreground">{issues.length}</span>
      </Button>
    </div>
  );
}

/**
 * Die kleinen Einträge unter „Hidden columns“ zählen nur, wenn der Zeiger darauf steht; sonst
 * gewänne bei `closestCorners` oft eine Karte der Nachbarspalte.
 */
const collisionDetection: CollisionDetection = (args) => {
  const isEntry = (c: { id: string | number }) => String(c.id).startsWith(HIDDEN_PREFIX);
  const entries = pointerWithin({
    ...args,
    droppableContainers: args.droppableContainers.filter(isEntry),
  });
  if (entries.length > 0) return entries;
  return closestCorners({
    ...args,
    droppableContainers: args.droppableContainers.filter((c) => !isEntry(c)),
  });
};

/** Board: Spalten = Status, Drag & Drop ändert Status und Reihenfolge (`applyDrop`). */
export function IssueBoard({
  columns,
  visibility,
  ws,
  lookup,
  focusedId,
  onFocus,
  onDrop,
  onCreateInColumn,
  selection,
  display,
  movesCard = () => true,
}: BoardProps) {
  const [dragging, setDragging] = useState<Issue | null>(null);
  // Nach gültigem Drop (Statuswechsel/Umsortieren) entfällt die Rückflug-Animation des Overlays:
  // sie zielt auf die Kartenposition vor dem optimistischen Update, die Karte erschiene sonst
  // erst am Ursprung und spränge dann in die Zielspalte. Abbruch und Drop ohne Änderung animieren.
  const [skipDropAnimation, setSkipDropAnimation] = useState(false);
  const { shown: shownColumns, hidden: hiddenColumns, toggle, isHidden, hide, keep } = visibility;
  const columnMenu = (id: string) => {
    if (!hide || !keep) return undefined;
    return isHidden(id)
      ? { label: "Keep visible", hidden: true, run: () => keep(id) }
      : { label: "Hide column", hidden: false, run: () => hide(id) };
  };

  // Strg+←/→: fokussierte Karte eine Spalte weiter verschieben (B4.19).
  const moveFocused = (delta: 1 | -1) => {
    if (!focusedId) return false as const;
    const at = shownColumns.findIndex((c) => c.issues.some((i) => i.id === focusedId));
    const target = shownColumns[at + delta];
    if (at < 0 || !target) return false as const;
    const result = applyDrop(columns, focusedId, COLUMN_PREFIX + target.state.id);
    const issue = columns.flatMap((c) => c.issues).find((i) => i.id === focusedId);
    if (result.patch && issue) onDrop({ ...result, issue });
  };
  useShortcuts([
    {
      id: "board.move-right",
      keys: "mod+arrowright",
      context: "list",
      description: "Move card to next column",
      run: () => moveFocused(1),
    },
    {
      id: "board.move-left",
      keys: "mod+arrowleft",
      context: "list",
      description: "Move card to previous column",
      run: () => moveFocused(-1),
    },
  ]);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space"] },
    }),
  );

  const onDragStart = (e: DragStartEvent) => {
    const id = String(e.active.id);
    setSkipDropAnimation(false);
    setDragging(columns.flatMap((c) => c.issues).find((i) => i.id === id) ?? null);
  };
  const onDragEnd = (e: DragEndEvent) => {
    setDragging(null);
    const over = e.over ? String(e.over.id).replace(HIDDEN_PREFIX, COLUMN_PREFIX) : null;
    const result = applyDrop(columns, String(e.active.id), over);
    const issue = columns.flatMap((c) => c.issues).find((i) => i.id === result.issueId);
    setSkipDropAnimation(!!result.patch && !!issue && movesCard(result.patch));
    if (result.patch && issue) onDrop({ ...result, issue });
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={collisionDetection}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setDragging(null)}
    >
      {/* Scroll-Container (beide Richtungen); die innere Zeile wächst mit der längsten Spalte
          und streckt alle Spalten auf diese Höhe, damit jeder Spaltenkopf sticky bleibt. */}
      <div className="h-full overflow-auto p-4">
        <div className="flex min-h-full w-max items-stretch gap-3">
          {shownColumns.map((c) => (
            <Column
              key={c.state.id}
              column={c}
              ws={ws}
              lookup={lookup}
              focusedId={focusedId}
              onFocus={onFocus}
              onCreate={onCreateInColumn}
              selection={selection}
              display={display}
              menu={columnMenu(c.state.id)}
            />
          ))}
          {shownColumns.length === 0 && (
            <p className="w-72 px-1 py-4 text-center text-sm text-muted-foreground">
              All columns are hidden
            </p>
          )}
          {hiddenColumns.length > 0 && (
            <aside
              aria-label="Hidden columns"
              className="flex w-56 shrink-0 flex-col gap-1 self-start rounded-xl bg-surface p-2"
            >
              <h2 className="px-2 py-1 text-sm font-medium">Hidden columns</h2>
              {hiddenColumns.map((entry, i) => (
                <Fragment key={entry.column.state.id}>
                  {entry.reason === "empty" && hiddenColumns[i - 1]?.reason !== "empty" && (
                    <h3 className="px-2 pt-2 text-xs text-subtle-foreground">Empty</h3>
                  )}
                  <HiddenColumnEntry entry={entry} onToggle={toggle} />
                </Fragment>
              ))}
            </aside>
          )}
        </div>
      </div>
      <DragOverlay dropAnimation={skipDropAnimation ? null : undefined}>
        {dragging && (
          <div className="rounded-lg border border-border-strong bg-card p-2.5 shadow-popover">
            <IssueCardBody issue={dragging} lookup={lookup} display={display} />
          </div>
        )}
      </DragOverlay>
    </DndContext>
  );
}
