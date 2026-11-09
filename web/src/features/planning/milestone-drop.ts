import { DROP_PREFIX } from "@/features/views/issue-view";

/**
 * Ablage-IDs fürs Ziehen „Ticket → Meilenstein" (LIN-46). Listengruppen und Seitenpanel sind zwei
 * getrennte Ablageflächen in **einer** DndContext (Projekt-Seite). @dnd-kit hält Droppables in einer
 * Map je ID; gleiche IDs auf beiden Flächen würden sich gegenseitig überschreiben. Darum führt das
 * Panel einen eigenen Präfix. `DROP_PREFIX` (Listengruppen) stammt aus der Issue-Ansicht.
 */
export const PANEL_DROP_PREFIX = "panel-drop:";

/** Meilenstein-Schlüssel aus einer Ablage-ID (beide Präfixe), sonst `null`. „none" = ohne Meilenstein. */
export function dropKeyFromId(overId: string | null | undefined): string | null {
  if (!overId) return null;
  if (overId.startsWith(DROP_PREFIX)) return overId.slice(DROP_PREFIX.length);
  if (overId.startsWith(PANEL_DROP_PREFIX)) return overId.slice(PANEL_DROP_PREFIX.length);
  return null;
}
