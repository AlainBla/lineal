import { Link } from "@tanstack/react-router";
import { useEffect, type ReactNode } from "react";

import { cn } from "@/lib/cn";

export function SettingsShell({
  title,
  nav,
  children,
}: {
  title: string;
  nav: Array<{ label: string; to: string; active: boolean }>;
  children: ReactNode;
}) {
  useEffect(() => {
    const prev = document.title;
    document.title = `${title} · Settings · Lineal`;
    return () => {
      document.title = prev;
    };
  }, [title]);
  return (
    <div className="flex h-full min-h-0">
      <nav
        aria-label="Settings sections"
        className="w-52 shrink-0 overflow-auto border-r border-border p-2"
      >
        <h1 className="px-2 py-1.5 text-sm font-semibold">{title}</h1>
        <ul className="flex flex-col gap-px">
          {nav.map((n) => (
            <li key={n.label}>
              <Link
                to={n.to as "/"}
                aria-current={n.active ? "page" : undefined}
                className={cn(
                  "block rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-surface-hover hover:text-foreground",
                  n.active && "bg-surface-active text-foreground",
                )}
              >
                {n.label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <div className="min-w-0 flex-1 overflow-auto">
        <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6">{children}</div>
      </div>
    </div>
  );
}

export function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-base font-semibold">{title}</h2>
        {description && <p className="text-sm text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  );
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

export const selectClass = "h-8 rounded-md border border-input bg-transparent px-2 text-sm";
