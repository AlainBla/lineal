import { Link } from "@tanstack/react-router";
import type { ComponentProps, ReactNode } from "react";

import { cn } from "@/lib/cn";

/**
 * Link auf einen Pfad, der (noch) nicht als Route existiert, weil ihn ein späteres WP baut.
 * Aktiver Zustand per `data-status="active"` (TanStack Router) und `aria-current="page"`.
 */
export function NavLink({
  to,
  className,
  children,
  ...props
}: { to: string; children: ReactNode } & Omit<ComponentProps<"a">, "href">) {
  return (
    <Link
      // Pfade späterer WPs sind dem Router-Typsystem noch unbekannt.
      to={to as "/"}
      className={cn(
        "group flex h-7 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground/85 transition-colors duration-[var(--duration-fast)] hover:bg-surface-hover hover:text-foreground data-[status=active]:bg-surface-active data-[status=active]:text-foreground [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground data-[status=active]:[&_svg]:text-foreground",
        className,
      )}
      activeOptions={{ exact: false }}
      {...props}
    >
      {children}
    </Link>
  );
}
