/**
 * Merkt sich den fokussierten Eintrag einer Liste pro Router und Adresse. Öffnet man ein Issue und
 * kehrt zurück (Esc / Zurück), wird die Liste neu gemountet – so bleibt die Zeile hervorgehoben.
 */
import { useCallback, useState } from "react";
import { useRouter } from "@tanstack/react-router";

const store = new WeakMap<object, Map<string, string | null>>();

export function useRememberedFocus(): [string | null, (id: string | null) => void] {
  const router = useRouter({ warn: false }) as ReturnType<typeof useRouter> | undefined;
  const [focusedId, setFocusedState] = useState<string | null>(() =>
    router ? (store.get(router)?.get(router.state.location.href) ?? null) : null,
  );
  const setFocusedId = useCallback(
    (id: string | null) => {
      setFocusedState(id);
      if (!router) return;
      let m = store.get(router);
      if (!m) store.set(router, (m = new Map()));
      m.set(router.state.location.href, id);
    },
    [router],
  );
  return [focusedId, setFocusedId];
}
