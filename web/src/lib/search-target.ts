/**
 * „Primäres Suchfeld“ einer Seite: Eingabefelder mit `data-search-target` werden von Cmd/Ctrl+F
 * angesteuert (statt der Browsersuche). Sichtbar heißt hier: nicht deaktiviert und nicht in einem
 * `hidden`/`inert`-Container.
 */
export const SEARCH_TARGET_ATTR = "data-search-target";

/** Bitte an die Topbar, das (auf schmalen Fenstern eingeklappte) Suchfeld aufzuklappen und zu fokussieren. */
export const OPEN_SEARCH_EVENT = "lineal:open-search";

function findSearchTarget(root: ParentNode): HTMLInputElement | HTMLTextAreaElement | null {
  const nodes = root.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
    `[${SEARCH_TARGET_ATTR}]`,
  );
  for (const el of nodes) {
    if (el.disabled || el.closest("[hidden],[inert]")) continue;
    return el;
  }
  return null;
}

/** Hat das Suchfeld der Seite bereits den Fokus? (Dann greift die Browsersuche.) */
export function isSearchTargetFocused(root: Document = document): boolean {
  const el = findSearchTarget(root);
  return el !== null && root.activeElement === el;
}

/** Fokussiert das erste sichtbare Suchfeld der Seite; `false`, wenn die Seite keins hat. */
export function focusSearchTarget(root: Document = document): boolean {
  if (typeof window !== "undefined") window.dispatchEvent(new Event(OPEN_SEARCH_EVENT));
  const el = findSearchTarget(root);
  if (!el) return false;
  el.focus();
  el.select();
  return true;
}
