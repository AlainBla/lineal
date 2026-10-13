import { useCallback, useRef } from "react";

/**
 * Submit-Sperre gegen Doppelsendungen (Doppelklick, mehrfaches Strg+Enter/Enter). Die Sperre ist ein
 * Ref und greift sofort, nicht erst nach dem nächsten Render wie `isPending`. Läuft bereits ein
 * Aufruf, liefert `run` `undefined`, ohne `fn` auszuführen; sonst das Ergebnis von `fn`.
 * Die Sperre wird auch bei einem Fehler wieder freigegeben.
 */
export function useSingleFlight(): <T>(fn: () => Promise<T>) => Promise<T | undefined> {
  const busy = useRef(false);
  return useCallback(async <T>(fn: () => Promise<T>): Promise<T | undefined> => {
    if (busy.current) return undefined;
    busy.current = true;
    try {
      return await fn();
    } finally {
      busy.current = false;
    }
  }, []);
}
