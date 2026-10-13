/**
 * Filterleiste und Display options an den Team-Seiten Active/Backlog/All (F.18, docs/05 §1.6, §2).
 * Filter laufen serverseitig (`viewIssues` mit Team-Scope), Anzeigeoptionen werden je Team und
 * Ansicht persönlich gespeichert (`viewPreferences` mit `viewType` und `teamId`). Tasten:
 * `F` Add filter, `⇧ V` Display options.
 */
import { useMemo, useRef, useState } from "react";

import { useActions } from "@/app/commands";
import type { IssueLookup } from "./issue-lookup";
import { IssueRow } from "./issue-row";
import { DisplayMenu } from "@/features/views/display-menu";
import {
  DEFAULT_DISPLAY,
  type IssueGroup,
  mergeDisplay,
  type DisplayOptions,
} from "@/features/views/display-options";
import { FilterBar } from "@/features/views/filter-bar";
import { buildFilter, type Filter, type FilterChip } from "@/features/views/filter-model";
import { combineFilter } from "@/features/views/filter-parse";
import {
  usePreferences,
  useResetPreferences,
  useUpdatePreferences,
  type PrefTarget,
} from "@/features/views/queries";
import { useFilterSources } from "@/features/views/use-filter-sources";

import type { TeamView } from "./model";

const VIEW_TYPE: Record<TeamView, string> = {
  active: "activeIssues",
  backlog: "backlog",
  all: "allIssues",
};

export interface TeamViewControlsState {
  chips: FilterChip[];
  setChips: (c: FilterChip[]) => void;
  advanced: Filter[];
  setAdvanced: (f: Filter[]) => void;
  /** Gesetzter Filter (leer = keiner). */
  filter: Filter;
  active: boolean;
  options: DisplayOptions;
  /** `true`, wenn die Anzeige von der Standarddarstellung (Status-Gruppen, manuell sortiert) abweicht. */
  customDisplay: boolean;
  onOptions: (patch: Partial<DisplayOptions>) => void;
  onReset: () => void;
}

/** Team-Standard: Gruppen nach Status, manuelle Reihenfolge (der Server liefert `sortOrder`). */
const TEAM_DEFAULT: DisplayOptions = { ...DEFAULT_DISPLAY, ordering: "manual" };

export function useTeamViewControls(
  teamId: string | undefined,
  view: TeamView,
): TeamViewControlsState {
  const [chips, setChips] = useState<FilterChip[]>([]);
  const [advanced, setAdvanced] = useState<Filter[]>([]);
  const [local, setLocal] = useState<Partial<DisplayOptions>>({});
  const target: PrefTarget | null = teamId ? { viewType: VIEW_TYPE[view], teamId } : null;
  const prefs = usePreferences(target);
  const update = useUpdatePreferences(target);
  const reset = useResetPreferences(target);
  const options = useMemo(() => {
    const saved = prefs.data?.values as Record<string, unknown> | undefined;
    const merged = mergeDisplay(saved);
    return {
      ...merged,
      ordering: saved && "ordering" in saved ? merged.ordering : TEAM_DEFAULT.ordering,
      ...local,
    };
  }, [prefs.data, local]);
  const filter = useMemo(() => combineFilter(buildFilter(chips), advanced), [chips, advanced]);
  const customDisplay =
    options.grouping !== "status" ||
    options.ordering !== "manual" ||
    !options.showSubIssues ||
    options.showCompletedIssues !== "all";
  return {
    chips,
    setChips,
    advanced,
    setAdvanced,
    filter,
    active: chips.length > 0 || advanced.length > 0,
    options,
    customDisplay,
    onOptions: (patch) => {
      setLocal((l) => ({ ...l, ...patch }));
      if (target) update.mutate(patch);
    },
    onReset: () => {
      setLocal({});
      reset.mutate();
    },
  };
}

/** Öffnet das Radix-Menü bzw. den Dialog hinter einem Knopf (Radix reagiert auf `pointerdown`). */
function press(el: Element | null | undefined) {
  if (!el) return;
  el.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0, cancelable: true }));
  (el as HTMLElement).click();
}

export function TeamViewControls({ controls }: { controls: TeamViewControlsState }) {
  const { sources } = useFilterSources();
  const root = useRef<HTMLDivElement>(null);
  useActions([
    {
      id: "view.add-filter",
      title: "Add filter…",
      section: "View",
      keys: "f",
      context: "list",
      perform: () => press(root.current?.querySelector('button[aria-label="Add filter"]')),
    },
    {
      id: "view.display-options",
      title: "Display options…",
      section: "View",
      keys: "shift+v",
      context: "list",
      perform: () => press(root.current?.querySelector('button[aria-haspopup="dialog"]')),
    },
  ]);
  return (
    <div ref={root} className="flex flex-wrap items-center gap-1">
      <FilterBar
        chips={controls.chips}
        onChange={controls.setChips}
        sources={sources}
        advanced={controls.advanced}
        onAdvancedChange={controls.setAdvanced}
      />
      <DisplayMenu
        options={controls.options}
        onChange={controls.onOptions}
        onReset={controls.onReset}
      />
    </div>
  );
}

/** Liste mit frei gewählter Gruppierung (Assignee, Projekt, Priorität, …); Zeilen wie in der Status-Liste. */
export function GroupedIssueList({
  groups,
  ws,
  lookup,
  focusedId,
  onFocus,
}: {
  groups: IssueGroup[];
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
}) {
  return (
    <div role="list" aria-label="Issues">
      {groups.map((g) => (
        <section key={g.key} aria-label={g.label}>
          <h2 className="flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm font-medium">
            {g.label}
            <span className="text-xs text-muted-foreground">{g.issues.length}</span>
          </h2>
          {g.issues.map((issue) => (
            <IssueRow
              key={issue.id}
              issue={issue}
              ws={ws}
              lookup={lookup}
              focused={focusedId === issue.id}
              onFocus={onFocus}
            />
          ))}
        </section>
      ))}
    </div>
  );
}
