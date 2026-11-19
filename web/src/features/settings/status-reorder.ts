import { arrayMove } from "@dnd-kit/sortable";

import { between } from "@/features/issues/model";

/**
 * Neue `position` eines per Drag-and-Drop verschobenen Status (LIN-73): Zwischenwert aus den neuen
 * Nachbarn derselben Kategorie (wie bei Meilensteinen LIN-47, `between`). Sortiert wird nur innerhalb
 * einer Statuskategorie – ein Ziel in einer anderen Kategorie ändert die Semantik des Status und wird
 * daher ignoriert (`null`). `null` auch, wenn nichts zu tun ist. Rein, damit testbar.
 */
export function statusDropPosition(
  states: ReadonlyArray<{ id: string; type: string; position: number }>,
  activeId: string,
  overId: string | null,
): number | null {
  if (!overId || activeId === overId) return null;
  const active = states.find((s) => s.id === activeId);
  const over = states.find((s) => s.id === overId);
  if (!active || !over) return null;
  // Nur innerhalb derselben Kategorie verschieben (Standard laut Ticket).
  if (active.type !== over.type) return null;
  const same = states.filter((s) => s.type === active.type).sort((a, b) => a.position - b.position);
  const oldIndex = same.findIndex((s) => s.id === activeId);
  const newIndex = same.findIndex((s) => s.id === overId);
  if (oldIndex < 0 || newIndex < 0) return null;
  const reordered = arrayMove([...same], oldIndex, newIndex);
  const pos = reordered.findIndex((s) => s.id === activeId);
  return between(reordered[pos - 1]?.position, reordered[pos + 1]?.position);
}
