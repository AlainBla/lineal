import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  LayoutGrid,
  List,
  SlidersHorizontal,
} from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/cn";

import {
  COMPLETED_WINDOWS,
  DISPLAY_PROPERTIES,
  GROUPINGS,
  ORDERINGS,
  defaultDirection,
  type DisplayOptions,
  type DisplayProperty,
} from "./display-options";

const LABEL: Record<string, string> = {
  none: "No grouping",
  status: "Status",
  assignee: "Assignee",
  project: "Project",
  priority: "Priority",
  team: "Team",
  cycle: "Cycle",
  milestone: "Milestone",
  title: "Title",
  updatedAt: "Last updated",
  createdAt: "Created",
  dueDate: "Due date",
  manual: "Manual",
  estimate: "Estimate",
  linkCount: "Link count",
  timeInStatus: "Time in status",
  all: "All",
  pastDay: "Past day",
  pastWeek: "Past week",
  pastMonth: "Past month",
  showAll: "Show all",
  showMatching: "Show matching",
  hide: "Hide",
};

const PROPERTY_LABEL: Record<DisplayProperty, string> = {
  id: "ID",
  status: "Status",
  assignee: "Assignee",
  creator: "Creator",
  priority: "Priority",
  dueDate: "Due date",
  milestone: "Milestone",
  cycle: "Cycle",
  labels: "Labels",
  links: "Links",
  timeInStatus: "Time in status",
  created: "Created",
  updated: "Updated",
};

const selectCls = "h-8 min-w-36 rounded-md border border-input bg-transparent px-2 text-sm";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-4 text-sm">
      <span>{label}</span>
      {children}
    </label>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <Row label={label}>
      <input
        type="checkbox"
        role="switch"
        aria-label={label}
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="h-4 w-8 cursor-pointer accent-primary"
      />
    </Row>
  );
}

/**
 * Anzeigeoptionen (docs/05 §2.1) als Popover: Layout, Gruppierung/Untergruppierung, Sortierung mit
 * Richtung, erledigte Issues, Sub-Issues, Listenoptionen und Display properties. Jede Änderung ist
 * persönlich und wird sofort über `onChange` gemeldet; „Set default for everyone“ übernimmt die
 * wirksamen Optionen als Standard für alle (Rechte prüft der Server).
 */
export function DisplayMenu({
  options,
  onChange,
  onReset,
  onSetDefault,
  customized,
  compact,
  showLayout,
}: {
  options: DisplayOptions;
  onChange: (patch: Partial<DisplayOptions>) => void;
  onReset?: () => void;
  onSetDefault?: () => void;
  /** Punkt am Knopf, wenn die Anzeige vom Standard abweicht. */
  customized?: boolean;
  /** Nur Symbol statt „Display“. */
  compact?: boolean;
  /** List/Board-Umschalter anbieten (nur, wo die Ansicht beides darstellen kann). */
  showLayout?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const desc = options.orderDirection === "desc";
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size={compact ? "icon" : "sm"}
          aria-label={compact ? "Display options" : undefined}
          title="Display options"
          className="relative"
        >
          <SlidersHorizontal />
          {!compact && " Display"}
          {customized && (
            <span
              data-testid="display-customized"
              className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-primary"
            />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent aria-label="Display options" className="w-80">
        <div className="flex flex-col gap-3">
          {showLayout && (
            <div role="group" aria-label="Layout" className="grid grid-cols-2 gap-2">
              {(["list", "board"] as const).map((l) => (
                <Button
                  key={l}
                  variant="ghost"
                  size="sm"
                  aria-pressed={options.layout === l}
                  className={cn(
                    "border border-border",
                    options.layout === l && "bg-surface-active",
                  )}
                  onClick={() => onChange({ layout: l })}
                >
                  {l === "list" ? <List /> : <LayoutGrid />}
                  {l === "list" ? "List" : "Board"}
                </Button>
              ))}
            </div>
          )}
          <Row label="Grouping">
            <select
              aria-label="Grouping"
              className={selectCls}
              value={options.grouping}
              onChange={(e) => onChange({ grouping: e.target.value as DisplayOptions["grouping"] })}
            >
              {GROUPINGS.map((g) => (
                <option key={g} value={g}>
                  {LABEL[g]}
                </option>
              ))}
            </select>
          </Row>
          <Row label="Sub-grouping">
            <select
              aria-label="Sub-grouping"
              className={selectCls}
              value={options.subGrouping}
              disabled={options.grouping === "none"}
              onChange={(e) =>
                onChange({ subGrouping: e.target.value as DisplayOptions["subGrouping"] })
              }
            >
              {GROUPINGS.filter((g) => g !== options.grouping).map((g) => (
                <option key={g} value={g}>
                  {LABEL[g]}
                </option>
              ))}
            </select>
          </Row>
          <div className="flex items-center justify-between gap-4 text-sm">
            <label htmlFor="display-ordering">Ordering</label>
            <span className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="icon"
                aria-label="Order direction"
                title={desc ? "Descending" : "Ascending"}
                disabled={options.ordering === "manual"}
                onClick={() => onChange({ orderDirection: desc ? "asc" : "desc" })}
              >
                {desc ? <ArrowDownWideNarrow /> : <ArrowUpNarrowWide />}
              </Button>
              <select
                id="display-ordering"
                aria-label="Ordering"
                className={selectCls}
                value={options.ordering}
                onChange={(e) => {
                  const ordering = e.target.value as DisplayOptions["ordering"];
                  onChange({ ordering, orderDirection: defaultDirection(ordering) });
                }}
              >
                {ORDERINGS.map((g) => (
                  <option key={g} value={g}>
                    {LABEL[g] ?? "Priority"}
                  </option>
                ))}
              </select>
            </span>
          </div>
          <hr className="border-border" />
          <Row label="Completed issues">
            <select
              aria-label="Completed issues"
              className={selectCls}
              value={options.showCompletedIssues}
              onChange={(e) =>
                onChange({
                  showCompletedIssues: e.target.value as DisplayOptions["showCompletedIssues"],
                })
              }
            >
              {COMPLETED_WINDOWS.map((w) => (
                <option key={w} value={w}>
                  {w === "none" ? "None" : LABEL[w]}
                </option>
              ))}
            </select>
          </Row>
          <Toggle
            label="Show sub-issues"
            checked={options.showSubIssues}
            onChange={(showSubIssues) => onChange({ showSubIssues })}
          />
          <hr className="border-border" />
          <div className="text-sm font-medium">List options</div>
          <Row label="Nested sub-issues">
            <select
              aria-label="Nested sub-issues"
              className={selectCls}
              value={options.nestedSubIssues}
              onChange={(e) =>
                onChange({ nestedSubIssues: e.target.value as DisplayOptions["nestedSubIssues"] })
              }
            >
              {(["showMatching", "showAll", "hide"] as const).map((v) => (
                <option key={v} value={v}>
                  {LABEL[v]}
                </option>
              ))}
            </select>
          </Row>
          <Toggle
            label="Show empty groups"
            checked={options.showEmptyGroups}
            onChange={(showEmptyGroups) => onChange({ showEmptyGroups })}
          />
          <hr className="border-border" />
          <div className="text-sm font-medium">Display properties</div>
          <div role="group" aria-label="Display properties" className="flex flex-wrap gap-1.5">
            {DISPLAY_PROPERTIES.map((p) => {
              const on = options.displayProperties.includes(p);
              return (
                <button
                  key={p}
                  type="button"
                  aria-pressed={on}
                  className={cn(
                    "rounded-full border border-border px-2.5 py-0.5 text-sm",
                    on ? "bg-surface-active" : "text-muted-foreground hover:bg-surface-hover",
                  )}
                  onClick={() =>
                    onChange({
                      displayProperties: DISPLAY_PROPERTIES.filter((x) =>
                        x === p ? !on : options.displayProperties.includes(x),
                      ),
                    })
                  }
                >
                  {PROPERTY_LABEL[p]}
                </button>
              );
            })}
          </div>
          {(onReset || onSetDefault) && (
            <div className="flex items-center justify-end gap-2 border-t border-border pt-2">
              {onReset && (
                <Button variant="ghost" size="sm" onClick={onReset}>
                  Reset
                </Button>
              )}
              {onSetDefault && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-primary"
                  onClick={() => {
                    onSetDefault();
                  }}
                >
                  Set default for everyone
                </Button>
              )}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
