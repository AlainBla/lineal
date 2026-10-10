import { useRouterState } from "@tanstack/react-router";
import { Construction } from "lucide-react";
import { motion } from "motion/react";
import { useEffect } from "react";

import { titleForPath } from "@/app/nav";

/** Leere Platzhalterseite für Bereiche, die spätere Arbeitspakete bauen. */
export function PlaceholderPage({ title }: { title?: string }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const heading = title ?? titleForPath(pathname);

  useEffect(() => {
    const previous = document.title;
    document.title = `${heading} · Lineal`;
    return () => {
      document.title = previous;
    };
  }, [heading]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center"
    >
      <div className="flex size-10 items-center justify-center rounded-lg border border-border-strong bg-surface text-muted-foreground">
        <Construction className="size-5" aria-hidden />
      </div>
      <h1 className="text-base font-semibold tracking-tight">{heading}</h1>
      <p className="max-w-sm text-sm text-muted-foreground">
        This area is not built yet. It will appear here once its feature is available.
      </p>
    </motion.div>
  );
}
