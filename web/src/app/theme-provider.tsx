import { MotionConfig } from "motion/react";
import { useEffect, type ReactNode } from "react";

import { applyTheme, getTheme, watchSystemTheme } from "@/lib/theme";

// Theme vor dem ersten Render setzen, damit es kein Aufblitzen im falschen Theme gibt.
if (typeof document !== "undefined") applyTheme(getTheme());

/** Hält `class="dark"` synchron (Systemwechsel) und respektiert `prefers-reduced-motion` für Motion. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  useEffect(() => {
    applyTheme(getTheme());
    return watchSystemTheme();
  }, []);
  return <MotionConfig reducedMotion="user">{children}</MotionConfig>;
}
