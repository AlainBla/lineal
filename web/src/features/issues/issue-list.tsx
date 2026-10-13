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
import { ChevronDown, Plus } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";

import { StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { IssueRow } from "./issue-row";
import { applyDrop, COLUMN_PREFIX, type BoardColumn, type DropResult } from "./model";
import type { Issue } from "@/gql/graphql";
import type { Selection } from "./selection";

interface ListProps {
  groups: BoardColumn[];
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  /** „+“ im Gruppenkopf: Issue mit Gruppenstatus anlegen. */
  onCreateInGroup?: (stateId: string) => void;
  selection?: Selection;
  /**
   * Drag & Drop zum Umsortieren (nur bei manueller Sortierung, docs/05 §2.5): innerhalb der Gruppe
   * ändert es `sortOrder`, in eine andere Gruppe zusätzlich den Status.
   */
  onDrop?: (result: DropResult & { issue: Issue }) => void;
}

function SortableRow({ id, children }: { id: string; children: ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id,
  });
  return (
    <div
      ref={setNodeRef}
      data-sortable-id={id}
      {...attributes}
      role="listitem"
      {...listeners}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(isDragging && "opacity-40")}
    >
      {children}
    </div>
  );
}

function GroupDrop({ id, children }: { id: string; children: ReactNode }) {
  const { setNodeRef, isOver } = useDroppable({ id });
  return (
    <div ref={setNodeRef} className={cn("min-h-2", isOver && "bg-surface-hover")}>
      {children}
    </div>
  );
}

/** Liste nach Status gruppiert (einklappbar, `+` je Gruppe). */
export function IssueList({
  groups,
  ws,
  lookup,
  focusedId,
  onFocus,
  onCreateInGroup,
  selection,
  onDrop,
}: ListProps) {
  const [dragging, setDragging] = useState<Issue | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space"] },
    }),
  );
  const all = groups.flatMap((g) => g.issues);
  const onDragEnd = (e: DragEndEvent) => {
    setDragging(null);
    const result = applyDrop(groups, String(e.active.id), e.over ? String(e.over.id) : null);
    const issue = all.find((i) => i.id === result.issueId);
    if (result.patch && issue) onDrop?.({ ...result, issue });
  };
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const list = (
    <div role="list" aria-label="Issues">
      {groups.map(({ state, issues }) => {
        const closed = collapsed.has(state.id);
        return (
          <section key={state.id} aria-label={state.name}>
            <header className="flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm">
              <button
                type="button"
                className="flex items-center gap-2 rounded-sm outline-none focus-visible:outline-2"
                aria-expanded={!closed}
                aria-label={`${closed ? "Expand" : "Collapse"} ${state.name}`}
                onClick={() => toggle(state.id)}
              >
                <ChevronDown
                  className={cn(
                    "size-3.5 text-muted-foreground transition-transform",
                    closed && "-rotate-90",
                  )}
                  aria-hidden
                />
                <StatusIcon type={state.type} color={state.color} />
                <span className="font-medium">{state.name}</span>
                <span className="text-xs text-muted-foreground">{issues.length}</span>
              </button>
              {onCreateInGroup && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="ml-auto size-6"
                  aria-label={`Create issue in ${state.name}`}
                  onClick={() => onCreateInGroup(state.id)}
                >
                  <Plus className="size-3.5" />
                </Button>
              )}
            </header>
            {!closed &&
              (onDrop ? (
                <GroupDrop id={COLUMN_PREFIX + state.id}>
                  <SortableContext
                    items={issues.map((i) => i.id)}
                    strategy={verticalListSortingStrategy}
                  >
                    {issues.map((issue) => (
                      <SortableRow key={issue.id} id={issue.id}>
                        <IssueRow
                          issue={issue}
                          ws={ws}
                          lookup={lookup}
                          focused={focusedId === issue.id}
                          onFocus={onFocus}
                          selection={selection}
                        />
                      </SortableRow>
                    ))}
                  </SortableContext>
                </GroupDrop>
              ) : (
                issues.map((issue) => (
                  <div role="listitem" key={issue.id}>
                    <IssueRow
                      issue={issue}
                      ws={ws}
                      lookup={lookup}
                      focused={focusedId === issue.id}
                      onFocus={onFocus}
                      selection={selection}
                    />
                  </div>
                ))
              ))}
          </section>
        );
      })}
    </div>
  );
  if (!onDrop) return list;
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={(e: DragStartEvent) =>
        setDragging(all.find((i) => i.id === String(e.active.id)) ?? null)
      }
      onDragEnd={onDragEnd}
      onDragCancel={() => setDragging(null)}
    >
      {list}
      <DragOverlay>
        {dragging && (
          <div className="rounded-md border border-border-strong bg-card px-4 py-2 text-sm shadow-popover">
            <span className="mr-2 text-xs text-muted-foreground">{dragging.identifier}</span>
            {dragging.title}
          </div>
        )}
      </DragOverlay>
    </DndContext>
  );
}
