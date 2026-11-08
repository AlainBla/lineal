import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  ChevronDown,
  LayoutGrid,
  List,
  SlidersHorizontal,
} from "lucide-react";
import { useRef, useState } from "react";

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
  pastTwoWeeks: "Past 2 weeks",
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
  project: "Project",
  links: "Links",
  timeInStatus: "Time in status",
  created: "Created",
  updated: "Updated",
};

const selectCls = "h-8 min-w-36 rounded-md border border-input bg-transparent px-2 text-sm";

/**
 * Auswahl, deren Optionsliste bei Hover (und Klick/Fokus/Tastatur) als Flyout aufklappt (LIN-44) —
 * ersetzt das native `<select>`. Das innere Popover ist portaliert; der Aufrufer schützt das äußere
 * Display-Popover per `onPointerDownOutside`/`onFocusOutside` vor dem Schließen (`data-hoverselect`).
 */
function HoverSelect<T extends string>({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  disabled?: boolean;
  onChange: (v: T) => void;
}) {
  const [open, setOpen] = useState(false);
  const openTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const clear = () => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
  };
  const hoverOpen = () => {
    if (disabled) return;
    clear();
    openTimer.current = setTimeout(() => setOpen(true), 100);
  };
  const scheduleClose = () => {
    clearTimeout(openTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 150);
  };
  const current = options.find((o) => o.value === value)?.label ?? value;
  return (
    <Popover
      open={open && !disabled}
      onOpenChange={(o) => {
        clear();
        setOpen(o);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={label}
          aria-haspopup="listbox"
          aria-expanded={open}
          disabled={disabled}
          onMouseEnter={hoverOpen}
          onMouseLeave={scheduleClose}
          onClick={() => {
            clear();
            setOpen((v) => !v);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              clear();
              setOpen(true);
            }
          }}
          className={cn(selectCls, "flex items-center justify-between gap-2 disabled:opacity-50")}
        >
          <span className="truncate">{current}</span>
          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        data-hoverselect=""
        role="listbox"
        aria-label={label}
        align="end"
        side="bottom"
        sideOffset={4}
        className="w-44 p-1"
        onMouseEnter={clear}
        onMouseLeave={scheduleClose}
        onKeyDown={(e) => {
          const items = Array.from(
            e.currentTarget.querySelectorAll<HTMLElement>('[role="option"]'),
          );
          const idx = items.indexOf(document.activeElement as HTMLElement);
          if (e.key === "ArrowDown") {
            e.preventDefault();
            items[Math.min(idx + 1, items.length - 1)]?.focus();
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            items[Math.max(idx - 1, 0)]?.focus();
          }
        }}
      >
        {options.map((o) => (
          <button
            key={o.value}
            type="button"
            role="option"
            aria-selected={o.value === value}
            className={cn(
              "flex w-full items-center rounded-md px-2 py-1.5 text-left text-sm hover:bg-surface-hover",
              o.value === value && "bg-surface-active",
            )}
            onClick={() => {
              onChange(o.value);
              clear();
              setOpen(false);
            }}
          >
            {o.label}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}

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
  boardOptions,
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
  /** Block „Board options“ anbieten (nur auf Boards, die versteckte Spalten kennen). */
  boardOptions?: boolean;
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
      <PopoverContent
        aria-label="Display options"
        className="w-80"
        onPointerDownOutside={(e) => {
          // LIN-44: Klick in ein offenes HoverSelect-Flyout schließt das Display-Popover nicht.
          if ((e.detail.originalEvent.target as Element | null)?.closest?.("[data-hoverselect]"))
            e.preventDefault();
        }}
        onFocusOutside={(e) => {
          if ((e.detail.originalEvent.target as Element | null)?.closest?.("[data-hoverselect]"))
            e.preventDefault();
        }}
      >
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
            <HoverSelect
              label="Grouping"
              value={options.grouping}
              options={GROUPINGS.map((g) => ({ value: g, label: LABEL[g] ?? g }))}
              onChange={(grouping) => onChange({ grouping })}
            />
          </Row>
          <Row label="Sub-grouping">
            <HoverSelect
              label="Sub-grouping"
              value={options.subGrouping}
              disabled={options.grouping === "none"}
              options={GROUPINGS.filter((g) => g !== options.grouping).map((g) => ({
                value: g,
                label: LABEL[g] ?? g,
              }))}
              onChange={(subGrouping) => onChange({ subGrouping })}
            />
          </Row>
          <div className="flex items-center justify-between gap-4 text-sm">
            <span>Ordering</span>
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
              <HoverSelect
                label="Ordering"
                value={options.ordering}
                options={ORDERINGS.map((g) => ({ value: g, label: LABEL[g] ?? "Priority" }))}
                onChange={(ordering) =>
                  onChange({ ordering, orderDirection: defaultDirection(ordering) })
                }
              />
            </span>
          </div>
          <hr className="border-border" />
          <Row label="Completed issues">
            <HoverSelect
              label="Completed issues"
              value={options.showCompletedIssues}
              options={COMPLETED_WINDOWS.map((w) => ({
                value: w,
                label: w === "none" ? "None" : (LABEL[w] ?? w),
              }))}
              onChange={(showCompletedIssues) => onChange({ showCompletedIssues })}
            />
          </Row>
          <Toggle
            label="Show sub-issues"
            checked={options.showSubIssues}
            onChange={(showSubIssues) => onChange({ showSubIssues })}
          />
          <hr className="border-border" />
          <div className="text-sm font-medium">List options</div>
          <Row label="Nested sub-issues">
            <HoverSelect
              label="Nested sub-issues"
              value={options.nestedSubIssues}
              options={(["showMatching", "showAll", "hide"] as const).map((v) => ({
                value: v,
                label: LABEL[v] ?? v,
              }))}
              onChange={(nestedSubIssues) => onChange({ nestedSubIssues })}
            />
          </Row>
          <Toggle
            label="Show empty groups"
            checked={options.showEmptyGroups}
            onChange={(showEmptyGroups) => onChange({ showEmptyGroups })}
          />
          {boardOptions && (
            <>
              <hr className="border-border" />
              <div className="text-sm font-medium">Board options</div>
              <Toggle
                label="Hide empty columns"
                checked={options.hideEmptyColumns}
                onChange={(hideEmptyColumns) => onChange({ hideEmptyColumns })}
              />
            </>
          )}
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
