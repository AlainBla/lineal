import { useSyncExternalStore } from "react";

/** Reagiert auf eine Media-Query (SSR/jsdom ohne matchMedia: `false`). */
export function useMediaQuery(query: string): boolean {
  const subscribe = (cb: () => void) => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => {};
    const mq = window.matchMedia(query);
    mq.addEventListener("change", cb);
    return () => mq.removeEventListener("change", cb);
  };
  const get = () =>
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia(query).matches;
  return useSyncExternalStore(subscribe, get, () => false);
}

/** Schmaler Viewport (< 768 px): Sidebar wird zum Overlay. */
export const NARROW_QUERY = "(max-width: 767px)";
