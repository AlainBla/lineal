import { arrayMove } from "@dnd-kit/sortable";

import { between } from "@/features/issues/model";

/**
 * Neue `sortOrder` eines per Drag-and-Drop verschobenen Meilensteins (LIN-47): Zwischenwert aus den
 * neuen Nachbarn (wie bei Issues, `between`). `null`, wenn nichts zu tun ist. Rein, damit testbar.
 */
export function milestoneDropSortOrder(
  milestones: ReadonlyArray<{ id: string; sortOrder: number }>,
  activeId: string,
  overId: string | null,
): number | null {
  if (!overId || activeId === overId) return null;
  const oldIndex = milestones.findIndex((m) => m.id === activeId);
  const newIndex = milestones.findIndex((m) => m.id === overId);
  if (oldIndex < 0 || newIndex < 0) return null;
  const reordered = arrayMove([...milestones], oldIndex, newIndex);
  const pos = reordered.findIndex((m) => m.id === activeId);
  return between(reordered[pos - 1]?.sortOrder, reordered[pos + 1]?.sortOrder);
}
