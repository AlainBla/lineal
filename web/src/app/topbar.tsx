import { useRouterState } from "@tanstack/react-router";
import { Monitor, Moon, PanelLeft, PanelRight, Search, Sun } from "lucide-react";
import { Fragment, useEffect, useRef } from "react";

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
import { clearQuickFilter, setQuickFilter, useQuickFilterState } from "@/lib/quick-filter";
import { SEARCH_TARGET_ATTR } from "@/lib/search-target";
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

/**
 * Suchfeld oben rechts: filtert auf Listen-/Board-Seiten live die sichtbaren Issues (Kennung als Präfix,
 * Titel als Teilstring). Auf Seiten ohne Issue-Liste öffnet Enter das Command-Menü mit dem Text.
 * Cmd/Ctrl+F fokussiert das Feld (`data-search-target`), Esc leert und verlässt es.
 */
function QuickSearch() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { text, count } = useQuickFilterState();
  const ref = useRef<HTMLInputElement>(null);

  // Seitenwechsel verwirft den Filter.
  useEffect(() => {
    clearQuickFilter();
  }, [pathname]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Escape") {
      e.preventDefault();
      clearQuickFilter();
      ref.current?.blur();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (count === null) {
        const q = text.trim();
        clearQuickFilter();
        ref.current?.blur();
        toggleCommandMenu(true, q);
      } else {
        // Fokus in die Liste zurückgeben, damit j/k auf den gefilterten Zeilen arbeiten.
        ref.current?.blur();
      }
    }
  };

  return (
    <label className="hidden h-7 w-56 items-center gap-2 rounded-md border border-border-strong px-2 text-xs text-muted-foreground transition-colors duration-[var(--duration-fast)] focus-within:border-ring focus-within:text-foreground hover:bg-surface-hover sm:flex">
      <Search className="size-3.5 shrink-0" aria-hidden />
      <input
        ref={ref}
        {...{ [SEARCH_TARGET_ATTR]: "" }}
        type="text"
        aria-label="Search or run a command"
        placeholder="Search or run a command"
        autoComplete="off"
        spellCheck={false}
        value={text}
        onChange={(e) => setQuickFilter(e.target.value)}
        onKeyDown={onKeyDown}
        className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground"
      />
      {text.trim() && count !== null ? (
        <span className="shrink-0 tabular-nums" aria-live="polite">
          {count} {count === 1 ? "issue" : "issues"}
        </span>
      ) : (
        <Keys keys="mod+k" />
      )}
    </label>
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

      <QuickSearch />
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
