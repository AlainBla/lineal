import { ChevronRight, ListFilter, Plus, Search, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { PRIORITIES, STATE_TYPE_LABEL, STATE_TYPE_ORDER } from "@/features/issues/model";

import {
  describeChip,
  FIELD_LABEL,
  FIELD_OPS,
  newChipId,
  OP_LABEL,
  RELATION_FILTERS,
  type FilterChip,
  type FilterField,
  type FilterOp,
} from "./filter-model";

export interface FilterSources {
  users: Array<{ id: string; name: string }>;
  labels: Array<{ id: string; name: string }>;
  projects: Array<{ id: string; name: string }>;
  teams: Array<{ id: string; name: string }>;
  /** Workflow-Status aller Teams (Filter „Status“); `team` = Team-Key zur Unterscheidung. */
  states?: Array<{ id: string; name: string; team?: string }>;
  /** Meilensteine der Projekte (Filter „Project milestone“). */
  milestones?: Array<{ id: string; name: string }>;
}

export interface Option {
  value: string;
  label: string;
}

/** Kategorien des Filtermenüs in der Reihenfolge von Linear (docs/05 §1.5); „Dates“ klappt auf. */
const MENU_GROUPS: Array<Array<FilterField | "dates">> = [
  [
    "team",
    "state",
    "stateType",
    "assignee",
    "creator",
    "priority",
    "label",
    "relations",
    "dates",
    "milestone",
    "cycle",
    "addedToCycle",
  ],
  ["subscribers", "autoClosed", "content", "links"],
  ["project", "estimate", "parent", "slaStatus", "title"],
];
const DATE_FIELD_LIST: FilterField[] = [
  "dueDate",
  "createdAt",
  "updatedAt",
  "startedAt",
  "completedAt",
  "canceledAt",
];

/** Wertevorrat je Feld; leer = freie Eingabe (Datum, Text). */
export function fieldOptions(field: FilterField, s: FilterSources): Option[] {
  switch (field) {
    case "assignee":
    case "creator":
      return [
        { value: "me", label: "Current user" },
        ...(field === "assignee" ? [{ value: "none", label: "No assignee" }] : []),
        ...s.users.map((u) => ({ value: u.id, label: u.name })),
      ];
    case "stateType":
      return STATE_TYPE_ORDER.map((t) => ({ value: t, label: STATE_TYPE_LABEL[t] }));
    case "priority":
      return PRIORITIES.map((p) => ({ value: String(p.value), label: p.label }));
    case "label":
      return [
        { value: "none", label: "No labels" },
        ...s.labels.map((l) => ({ value: l.id, label: l.name })),
      ];
    case "project":
      return [
        { value: "none", label: "No project" },
        ...s.projects.map((p) => ({ value: p.id, label: p.name })),
      ];
    case "cycle":
      return [
        { value: "current", label: "Current cycle" },
        { value: "upcoming", label: "Upcoming cycle" },
        { value: "previous", label: "Previous cycle" },
        { value: "none", label: "No cycle" },
      ];
    case "team":
      return s.teams.map((t) => ({ value: t.id, label: t.name }));
    case "dueDate":
      return [{ value: "overdue", label: "Overdue" }];
    case "subscribers":
      return s.users.map((u) => ({ value: u.id, label: u.name }));
    case "slaStatus":
      return [
        { value: "breached", label: "Breached" },
        { value: "highRisk", label: "High risk" },
        { value: "mediumRisk", label: "Medium risk" },
        { value: "lowRisk", label: "Low risk" },
      ];
    case "parent":
      return [{ value: "none", label: "No parent issue" }];
    case "state": {
      const states = s.states ?? [];
      const dup = new Set(
        states
          .filter((x, i) => states.findIndex((y) => y.name === x.name) !== i)
          .map((x) => x.name),
      );
      return states.map((x) => ({
        value: x.id,
        label: dup.has(x.name) && x.team ? `${x.name} (${x.team})` : x.name,
      }));
    }
    case "milestone": {
      const seen = new Set<string>();
      return [
        { value: "none", label: "No milestone" },
        ...(s.milestones ?? [])
          .filter((m) => !seen.has(m.id) && !!seen.add(m.id))
          .map((m) => ({ value: m.id, label: m.name })),
      ];
    }
    case "relations":
      return Object.entries(RELATION_FILTERS).map(([value, r]) => ({ value, label: r.label }));
    case "addedToCycle":
      return [
        { value: "before", label: "Before cycle start" },
        { value: "during", label: "During cycle" },
        { value: "after", label: "After cycle start" },
      ];
    case "autoClosed":
      return [{ value: "yes", label: "Auto-closed" }];
    default:
      return [];
  }
}

function nameOf(field: FilterField, v: string, s: FilterSources): string {
  return fieldOptions(field, s).find((o) => o.value === v)?.label ?? v;
}

/** Freie Eingabe: Text, Zahl oder Datum (auch relative Dauern wie `-P1W`, dann als Text). */
function inputType(field: FilterField, text: string): "text" | "number" | "date" {
  if (field === "title" || field === "content" || field === "links") return "text";
  if (field === "estimate") return "number";
  return text.startsWith("-P") || text.startsWith("P") ? "text" : "date";
}

function AddFilterDialog({
  field,
  sources,
  initial,
  onClose,
  onAdd,
}: {
  field: FilterField;
  sources: FilterSources;
  /** Vorhandener Chip, der bearbeitet wird. */
  initial?: FilterChip;
  onClose: () => void;
  onAdd: (c: FilterChip) => void;
}) {
  const options = fieldOptions(field, sources);
  const ops = FIELD_OPS[field];
  const [op, setOp] = useState<FilterOp>(initial?.op ?? ops[0]!);
  const isFree = (o: FilterOp) =>
    options.length === 0 || ((o === "before" || o === "after") && field === "dueDate");
  const [values, setValues] = useState<string[]>(
    initial && !isFree(initial.op) ? initial.values : [],
  );
  const [text, setText] = useState(initial && isFree(initial.op) ? (initial.values[0] ?? "") : "");
  const [subTeams, setSubTeams] = useState(initial?.subTeams ?? true);
  const freeText = isFree(op);
  const toggle = (v: string) =>
    setValues((cur) => (cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v]));
  const final = freeText ? (text ? [text] : []) : values;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm" aria-describedby={undefined}>
        <DialogTitle>Filter by {FIELD_LABEL[field].toLowerCase()}</DialogTitle>
        <DialogDescription className="sr-only">Choose an operator and values.</DialogDescription>
        <div className="flex flex-col gap-3">
          {ops.length > 1 && (
            <select
              aria-label="Operator"
              value={op}
              onChange={(e) => setOp(e.target.value as FilterOp)}
              className="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
            >
              {ops.map((o) => (
                <option key={o} value={o}>
                  {OP_LABEL[o]}
                </option>
              ))}
            </select>
          )}
          {freeText ? (
            <Input
              aria-label="Value"
              type={inputType(field, text)}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          ) : (
            <ul className="flex max-h-60 flex-col overflow-auto" aria-label="Values">
              {options.map((o) => (
                <li key={o.value}>
                  <label className="flex items-center gap-2 rounded px-1 py-1 text-sm hover:bg-surface-hover">
                    <input
                      type="checkbox"
                      checked={values.includes(o.value)}
                      onChange={() => toggle(o.value)}
                    />
                    {o.label}
                  </label>
                </li>
              ))}
            </ul>
          )}
          {field === "team" && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={subTeams}
                onChange={(e) => setSubTeams(e.target.checked)}
              />
              Include sub-teams
            </label>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={final.length === 0}
              onClick={() => {
                onAdd({
                  id: initial?.id ?? newChipId(),
                  field,
                  op,
                  values: final,
                  ...(field === "team" ? { subTeams } : {}),
                });
                onClose();
              }}
            >
              Apply
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Filterchips (docs/05 §1.6) mit „x“; ein Klick auf den Chip öffnet den Wertedialog. */
export function FilterChips({
  chips,
  onChange,
  sources,
  advanced = [],
  onAdvancedChange,
  showClear = true,
}: {
  chips: FilterChip[];
  onChange: (chips: FilterChip[]) => void;
  sources: FilterSources;
  /** Nicht als Chip darstellbare Filterteile („erweiterter Filter“); bleiben unverändert aktiv. */
  advanced?: Array<Record<string, unknown>>;
  onAdvancedChange?: (rest: Array<Record<string, unknown>>) => void;
  showClear?: boolean;
}) {
  const [editing, setEditing] = useState<FilterChip | null>(null);
  return (
    <>
      {chips.map((c) => (
        <span
          key={c.id}
          data-chip
          className="inline-flex h-6 items-center gap-1 rounded-full border border-border bg-surface px-2 text-xs"
        >
          <button
            type="button"
            aria-label={`Edit filter ${FIELD_LABEL[c.field]}`}
            className="hover:underline"
            onClick={() => setEditing(c)}
          >
            {describeChip(c, (f, v) => nameOf(f, v, sources))}
          </button>
          <button
            type="button"
            aria-label={`Remove filter ${FIELD_LABEL[c.field]}`}
            className="rounded-full text-muted-foreground hover:text-foreground"
            onClick={() => onChange(chips.filter((x) => x.id !== c.id))}
          >
            <X className="size-3" />
          </button>
        </span>
      ))}
      {advanced.length > 0 && (
        <span
          data-chip
          data-advanced
          title={JSON.stringify(advanced, null, 1)}
          className="inline-flex h-6 items-center gap-1 rounded-full border border-dashed border-border bg-surface px-2 text-xs text-muted-foreground"
        >
          Advanced filter ({advanced.length})
          {onAdvancedChange && (
            <button
              type="button"
              aria-label="Remove advanced filter"
              className="rounded-full hover:text-foreground"
              onClick={() => onAdvancedChange([])}
            >
              <X className="size-3" />
            </button>
          )}
        </span>
      )}
      {showClear && (chips.length > 0 || advanced.length > 0) && (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            onChange([]);
            onAdvancedChange?.([]);
          }}
        >
          Clear
        </Button>
      )}
      {editing && (
        <AddFilterDialog
          key={editing.id}
          field={editing.field}
          initial={editing}
          sources={sources}
          onClose={() => setEditing(null)}
          onAdd={(c) => onChange(chips.map((x) => (x.id === c.id ? c : x)))}
        />
      )}
    </>
  );
}

/** Dialog für den erweiterten Filter: rohes Filter-JSON (docs/05 §1.6), der Server validiert es. */
function AdvancedFilterDialog({
  initial,
  onClose,
  onApply,
}: {
  initial: Array<Record<string, unknown>>;
  onClose: () => void;
  onApply: (rest: Array<Record<string, unknown>>) => void;
}) {
  const [text, setText] = useState(initial.length ? JSON.stringify(initial, null, 2) : "{\n  \n}");
  const [error, setError] = useState<string | null>(null);
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg" aria-describedby={undefined}>
        <DialogTitle>Advanced filter</DialogTitle>
        <DialogDescription className="sr-only">Edit the filter as JSON.</DialogDescription>
        <textarea
          aria-label="Filter JSON"
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={10}
          spellCheck={false}
          className="w-full rounded-md border border-input bg-transparent p-2 font-mono text-xs outline-none focus-visible:border-ring"
        />
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            onClick={() => {
              try {
                const v: unknown = JSON.parse(text);
                const list = Array.isArray(v) ? v : [v];
                if (!list.every((x) => typeof x === "object" && x !== null && !Array.isArray(x))) {
                  throw new Error("Expected a filter object.");
                }
                onApply(list.filter((x) => Object.keys(x as object).length > 0));
                onClose();
              } catch (e) {
                setError(e instanceof Error ? e.message : "Invalid JSON.");
              }
            }}
          >
            Apply
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Filtermenü („Add filter…“, Taste F): Suchfeld, „Advanced filter“ und die Kategorien aus docs/05 §1.5.
 * Eine Kategorie öffnet den Wertedialog (Operator + Werte); „Dates“ klappt die Datumsfelder auf.
 */
export function FilterMenu({
  chips,
  onChange,
  sources,
  advanced = [],
  onAdvancedChange,
  compact,
}: {
  chips: FilterChip[];
  onChange: (chips: FilterChip[]) => void;
  sources: FilterSources;
  advanced?: Array<Record<string, unknown>>;
  onAdvancedChange?: (rest: Array<Record<string, unknown>>) => void;
  /** Nur Symbol (mit Punkt, wenn Filter aktiv sind). */
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  // Innerhalb eines modalen Dialogs (View-Builder) ist das Popover selbst modal (verschachtelte Ebene).
  const [inDialog, setInDialog] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setInDialog(!!triggerRef.current?.closest('[role="dialog"]'));
  }, []);
  const [query, setQuery] = useState("");
  const [dates, setDates] = useState(false);
  const [adding, setAdding] = useState<FilterField | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const active = chips.length + advanced.length;
  const q = query.trim().toLowerCase();
  const matches = (f: FilterField) => !q || FIELD_LABEL[f].toLowerCase().includes(q);
  const pick = (f: FilterField) => {
    setOpen(false);
    setAdding(f);
  };
  const groups = MENU_GROUPS.map((g) =>
    g.flatMap((e): Array<FilterField | "dates"> => {
      if (e === "dates") {
        // Mit Suche werden die Datumsfelder direkt durchsucht.
        return q ? DATE_FIELD_LIST.filter(matches) : dates ? DATE_FIELD_LIST : ["dates"];
      }
      return matches(e) ? [e] : [];
    }),
  ).filter((g) => g.length > 0);
  const first = groups[0]?.[0];
  const item =
    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-surface-hover";
  return (
    <>
      <Popover
        modal={inDialog}
        open={open}
        onOpenChange={(o) => {
          setOpen(o);
          if (!o) {
            setQuery("");
            setDates(false);
          }
        }}
      >
        <PopoverTrigger asChild>
          <Button
            variant="ghost"
            size={compact ? "icon" : "sm"}
            aria-label="Add filter"
            title="Add filter (F)"
            className="relative"
            ref={triggerRef}
          >
            {compact || chips.length === 0 ? <ListFilter /> : <Plus />}
            {!compact && " Filter"}
            {compact && active > 0 && (
              <span className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-primary" />
            )}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-64 p-1" aria-label="Add filter">
          <div role="menu" aria-label="Filter categories" className="flex flex-col">
            <div className="flex items-center gap-2 border-b border-border px-2 pt-1 pb-1.5">
              <Search className="size-4 text-muted-foreground" />
              <input
                autoFocus
                aria-label="Search filters"
                placeholder="Add filter…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && first && first !== "dates") pick(first);
                }}
                className="h-6 flex-1 bg-transparent text-sm outline-none"
              />
              <kbd className="rounded border border-border px-1 text-xs text-muted-foreground">
                F
              </kbd>
            </div>
            {onAdvancedChange && (!q || "advanced filter".includes(q)) && (
              <button
                type="button"
                role="menuitem"
                className={item}
                onClick={() => {
                  setOpen(false);
                  setAdvancedOpen(true);
                }}
              >
                Advanced filter
              </button>
            )}
            {groups.map((g, i) => (
              <div key={i} className="mt-1 border-t border-border pt-1">
                {g.map((e) =>
                  e === "dates" ? (
                    <button
                      key="dates"
                      type="button"
                      role="menuitem"
                      className={item}
                      onClick={() => setDates(true)}
                    >
                      Dates <ChevronRight className="ml-auto size-3.5" />
                    </button>
                  ) : (
                    <button
                      key={e}
                      type="button"
                      role="menuitem"
                      className={item}
                      onClick={() => pick(e)}
                    >
                      {FIELD_LABEL[e]}
                      <ChevronRight className="ml-auto size-3.5" />
                    </button>
                  ),
                )}
              </div>
            ))}
            {groups.length === 0 && (
              <p className="px-2 py-2 text-xs text-muted-foreground">No matching filters.</p>
            )}
          </div>
        </PopoverContent>
      </Popover>
      {adding && (
        <AddFilterDialog
          field={adding}
          sources={sources}
          onClose={() => setAdding(null)}
          onAdd={(c) => onChange([...chips, c])}
        />
      )}
      {advancedOpen && onAdvancedChange && (
        <AdvancedFilterDialog
          initial={advanced}
          onClose={() => setAdvancedOpen(false)}
          onApply={onAdvancedChange}
        />
      )}
    </>
  );
}

/** Filterleiste (docs/05 §1.6): Chips, Filtermenü und „Clear“ in einer Zeile. */
export function FilterBar(props: {
  chips: FilterChip[];
  onChange: (chips: FilterChip[]) => void;
  sources: FilterSources;
  advanced?: Array<Record<string, unknown>>;
  onAdvancedChange?: (rest: Array<Record<string, unknown>>) => void;
}) {
  return (
    <div role="group" aria-label="Filters" className="flex flex-wrap items-center gap-1">
      <FilterChips {...props} showClear={false} />
      <FilterMenu {...props} />
      {(props.chips.length > 0 || (props.advanced?.length ?? 0) > 0) && (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            props.onChange([]);
            props.onAdvancedChange?.([]);
          }}
        >
          Clear
        </Button>
      )}
    </div>
  );
}
