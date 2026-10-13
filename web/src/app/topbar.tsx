import { useRouterState } from "@tanstack/react-router";
import { Monitor, Moon, PanelLeft, PanelRight, Search, Sun } from "lucide-react";
import { Fragment } from "react";

import { breadcrumbFor } from "@/app/nav";
import { toggleCommandMenu, toggleSidebar, setUiState, useUiState } from "@/app/ui-state";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Keys } from "@/components/ui/kbd";
import { Hint } from "@/components/ui/tooltip";
import { NARROW_QUERY, useMediaQuery } from "@/lib/use-media-query";
import { isThemePreference, useTheme } from "@/lib/theme";

function ThemeMenu() {
  const { preference, resolved, setTheme } = useTheme();
  const Icon = resolved === "dark" ? Moon : Sun;
  return (
    <DropdownMenu>
      <Hint label="Theme">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" aria-label={`Theme: ${preference}`}>
            <Icon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuLabel>Theme</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={preference}
          onValueChange={(v) => isThemePreference(v) && setTheme(v)}
        >
          <DropdownMenuRadioItem value="dark">
            <Moon /> Dark
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="light">
            <Sun /> Light
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="system">
            <Monitor /> System
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function Topbar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { sidebarOpen, detailsOpen } = useUiState();
  const crumbs = breadcrumbFor(pathname);
  const narrow = useMediaQuery(NARROW_QUERY);

  return (
    <header className="flex h-[var(--topbar-height)] shrink-0 items-center gap-2 border-b border-border px-3">
      <Hint label={sidebarOpen ? "Hide sidebar" : "Show sidebar"} keys="[" side="bottom">
        <Button
          variant="ghost"
          size="icon"
          aria-label="Toggle sidebar"
          aria-pressed={sidebarOpen}
          onClick={() => toggleSidebar(!narrow)}
        >
          <PanelLeft />
        </Button>
      </Hint>

      <nav aria-label="Breadcrumb" className="min-w-0 flex-1">
        <ol className="flex items-center gap-1.5 text-sm">
          {(crumbs.length ? crumbs : ["Home"]).map((c, i, all) => (
            <Fragment key={`${c}-${i}`}>
              {i > 0 && (
                <li aria-hidden className="text-subtle-foreground">
                  /
                </li>
              )}
              <li
                className={
                  i === all.length - 1 ? "truncate font-medium" : "truncate text-muted-foreground"
                }
                aria-current={i === all.length - 1 ? "page" : undefined}
              >
                {c}
              </li>
            </Fragment>
          ))}
        </ol>
      </nav>

      <button
        type="button"
        onClick={() => toggleCommandMenu(true)}
        className="hidden h-7 items-center gap-2 rounded-md border border-border-strong px-2 text-xs text-muted-foreground transition-colors duration-[var(--duration-fast)] hover:bg-surface-hover hover:text-foreground sm:flex"
      >
        <Search className="size-3.5" aria-hidden />
        Search or run a command
        <Keys keys="mod+k" />
      </button>
      <ThemeMenu />
      <Hint label={detailsOpen ? "Hide details" : "Show details"} keys="]" side="bottom">
        <Button
          variant="ghost"
          size="icon"
          aria-label="Toggle details"
          aria-pressed={detailsOpen}
          onClick={() => setUiState({ detailsOpen: !detailsOpen })}
        >
          <PanelRight />
        </Button>
      </Hint>
    </header>
  );
}
