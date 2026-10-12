/**
 * Reihenfolge der Favoriten (docs/05 §4.6): `sortOrder` ist ein Float, neue Position = Mittelwert der
 * Nachbarn; Ordner sind eine Ebene tief. Rein, damit Drag & Drop ohne DOM testbar ist.
 */
export interface FavoriteItem {
  id: string;
  type: string;
  parentId?: string | null;
  sortOrder: number;
}

export interface FavoriteMove {
  id: string;
  sortOrder: number;
  parentId: string | null;
}

/** Position zwischen zwei Nachbarn (`null` = Rand). */
export function between(before: number | null, after: number | null): number {
  if (before === null && after === null) return 0;
  if (before === null) return after! - 1;
  if (after === null) return before + 1;
  return (before + after) / 2;
}

const level = (items: FavoriteItem[], parent: string | null) =>
  items.filter((f) => (f.parentId ?? null) === parent).sort((a, b) => a.sortOrder - b.sortOrder);

/**
 * Ergebnis von „`activeId` wurde über `overId` abgelegt“:
 * - Nicht-Ordner auf einen fremden Ordner: ans Ende dieses Ordners.
 * - Kind auf den eigenen Ordner: auf die oberste Ebene direkt hinter den Ordner.
 * - sonst: an die Stelle von `overId` in dessen Ebene (Ordner nie in Ordner).
 * `null`, wenn sich nichts ändert.
 */
export function computeFavoriteMove(
  items: FavoriteItem[],
  activeId: string,
  overId: string,
): FavoriteMove | null {
  if (activeId === overId) return null;
  const active = items.find((f) => f.id === activeId);
  const over = items.find((f) => f.id === overId);
  if (!active || !over) return null;
  const activeParent = active.parentId ?? null;
  const isFolder = active.type === "folder";

  if (over.type === "folder" && !isFolder && activeParent !== over.id) {
    const kids = level(items, over.id);
    const last = kids.length ? kids[kids.length - 1]!.sortOrder : null;
    return { id: activeId, parentId: over.id, sortOrder: between(last, null) };
  }
  if (over.type === "folder" && activeParent === over.id) {
    const top = level(items, null).filter((f) => f.id !== activeId);
    const i = top.findIndex((f) => f.id === over.id);
    const next = top[i + 1]?.sortOrder ?? null;
    return { id: activeId, parentId: null, sortOrder: between(over.sortOrder, next) };
  }

  const targetParent = over.parentId ?? null;
  if (isFolder && targetParent !== null) return null;
  const siblings = level(items, targetParent).filter((f) => f.id !== activeId);
  const overIdx = siblings.findIndex((f) => f.id === over.id);
  // Beim Verschieben nach unten innerhalb einer Ebene hinter, nach oben vor das Ziel einsortieren.
  const sameLevel = activeParent === targetParent;
  const from = sameLevel ? level(items, targetParent).findIndex((f) => f.id === activeId) : -1;
  const to = level(items, targetParent).findIndex((f) => f.id === over.id);
  const after = sameLevel && from < to;
  const at = after ? overIdx + 1 : overIdx;
  const before = siblings[at - 1]?.sortOrder ?? null;
  const next = siblings[at]?.sortOrder ?? null;
  const sortOrder = between(before, next);
  if (sameLevel && sortOrder === active.sortOrder) return null;
  return { id: activeId, parentId: targetParent, sortOrder };
}
