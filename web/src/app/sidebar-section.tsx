/**
 * Einklappbarer Abschnitt der linken Leiste (wie in Linear: Klick auf die Überschrift klappt ein/aus).
 * Der Zustand bleibt pro Abschnitt im Browser gespeichert.
 */
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useId, useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";

const STORAGE_PREFIX = "lineal-sidebar-section:";

function readOpen(id: string): boolean {
  try {
    return localStorage.getItem(STORAGE_PREFIX + id) !== "false";
  } catch {
    return true;
  }
}

function writeOpen(id: string, open: boolean) {
  try {
    if (open) localStorage.removeItem(STORAGE_PREFIX + id);
    else localStorage.setItem(STORAGE_PREFIX + id, "false");
  } catch {
    /* ignorieren */
  }
}

export function CollapsibleSection({
  id,
  label,
  children,
}: {
  /** Schlüssel für den gespeicherten Zustand (z. B. `workspace`). */
  id: string;
  label: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(() => readOpen(id));
  const panelId = useId();
  const toggle = () => {
    setOpen((o) => {
      writeOpen(id, !o);
      return !o;
    });
  };
  return (
    <section aria-label={label}>
      <h2 className="px-1 pt-4 pb-1">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={panelId}
          onClick={toggle}
          className="group flex h-5 items-center gap-1 rounded-md px-1 text-xs font-medium text-subtle-foreground transition-colors duration-[var(--duration-fast)] select-none hover:bg-surface-hover hover:text-sidebar-foreground"
        >
          {label}
          <ChevronDown
            aria-hidden
            className={cn(
              "size-3 shrink-0 transition-[transform,opacity] duration-[var(--duration-base)]",
              open
                ? "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100"
                : "-rotate-90",
            )}
          />
        </button>
      </h2>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            id={panelId}
            key="panel"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
