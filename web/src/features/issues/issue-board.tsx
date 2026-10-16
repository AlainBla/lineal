import {
  closestCorners,
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import type { Issue } from "@/gql/graphql";
import { cn } from "@/lib/cn";
import { useShortcuts } from "@/lib/shortcuts";

import { StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { IssueCardBody, IssueCardHover, SelectBox } from "./issue-row";
import { applyDrop, COLUMN_PREFIX, issuePath, type BoardColumn, type DropResult } from "./model";
import type { Selection } from "./selection";

interface BoardProps {
  columns: BoardColumn[];
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  /** Wird mit dem Ergebnis eines gültigen Drops aufgerufen (Statuswechsel und/oder neue Reihenfolge). */
  onDrop: (result: DropResult & { issue: Issue }) => void;
  onCreateInColumn?: (stateId: string) => void;
  selection?: Selection;
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
}: {
  issue: Issue;
  ws: string;
  lookup: IssueLookup;
  focused: boolean;
  onFocus: (id: string) => void;
  selection?: Selection;
}) {
  const selected = selection?.has(issue.id) ?? false;
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: issue.id,
  });
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
      <IssueCardBody issue={issue} lookup={lookup} />
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
}: {
  column: BoardColumn;
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  onCreate?: (stateId: string) => void;
  selection?: Selection;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: COLUMN_PREFIX + column.state.id });
  const { state, issues } = column;
  return (
    <section
      aria-label={state.name}
      data-column={state.id}
      className="flex w-72 shrink-0 flex-col rounded-xl bg-surface"
    >
      <header className="flex h-9 items-center gap-2 px-3 text-sm">
        <StatusIcon type={state.type} color={state.color} />
        <span className="font-medium">{state.name}</span>
        <span className="text-xs text-muted-foreground">{issues.length}</span>
        {onCreate && (
          <Button
            variant="ghost"
            size="icon"
            className="ml-auto size-6"
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

/** Board: Spalten = Status, Drag & Drop ändert Status und Reihenfolge (`applyDrop`). */
export function IssueBoard({
  columns,
  ws,
  lookup,
  focusedId,
  onFocus,
  onDrop,
  onCreateInColumn,
  selection,
  movesCard = () => true,
}: BoardProps) {
  const [dragging, setDragging] = useState<Issue | null>(null);
  // Nach gültigem Drop (Statuswechsel/Umsortieren) entfällt die Rückflug-Animation des Overlays:
  // sie zielt auf die Kartenposition vor dem optimistischen Update, die Karte erschiene sonst
  // erst am Ursprung und spränge dann in die Zielspalte. Abbruch und Drop ohne Änderung animieren.
  const [skipDropAnimation, setSkipDropAnimation] = useState(false);
  // Canceled/Duplicate stehen unter „Hidden columns“ (docs/08 F5), standardmäßig eingeklappt.
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set());
  const isHiddenType = (c: BoardColumn) =>
    c.state.type === "canceled" || c.state.type === "duplicate";
  const hiddenColumns = columns.filter(isHiddenType);
  const shownColumns = columns.filter((c) => !isHiddenType(c) || revealed.has(c.state.id));
  const toggleRevealed = (id: string) =>
    setRevealed((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

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
    const result = applyDrop(columns, String(e.active.id), e.over ? String(e.over.id) : null);
    const issue = columns.flatMap((c) => c.issues).find((i) => i.id === result.issueId);
    setSkipDropAnimation(!!result.patch && !!issue && movesCard(result.patch));
    if (result.patch && issue) onDrop({ ...result, issue });
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setDragging(null)}
    >
      <div className="flex h-full items-start gap-3 overflow-x-auto p-4">
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
          />
        ))}
        {hiddenColumns.length > 0 && (
          <aside
            aria-label="Hidden columns"
            className="flex w-56 shrink-0 flex-col gap-1 rounded-xl bg-surface p-2"
          >
            <h2 className="px-2 py-1 text-sm font-medium">Hidden columns</h2>
            {hiddenColumns.map((c) => {
              const open = revealed.has(c.state.id);
              return (
                <Button
                  key={c.state.id}
                  variant="ghost"
                  size="sm"
                  className="justify-start gap-2"
                  aria-pressed={open}
                  aria-label={`${open ? "Hide" : "Show"} ${c.state.name} column`}
                  onClick={() => toggleRevealed(c.state.id)}
                >
                  <StatusIcon type={c.state.type} color={c.state.color} />
                  <span className="truncate">{c.state.name}</span>
                  <span className="ml-auto text-xs text-muted-foreground">{c.issues.length}</span>
                </Button>
              );
            })}
          </aside>
        )}
      </div>
      <DragOverlay dropAnimation={skipDropAnimation ? null : undefined}>
        {dragging && (
          <div className="rounded-lg border border-border-strong bg-card p-2.5 shadow-popover">
            <IssueCardBody issue={dragging} lookup={lookup} />
          </div>
        )}
      </DragOverlay>
    </DndContext>
  );
}
