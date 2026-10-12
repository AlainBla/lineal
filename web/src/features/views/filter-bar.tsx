import { Filter as FilterIcon, Plus, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { PRIORITIES, STATE_TYPE_LABEL, STATE_TYPE_ORDER } from "@/features/issues/model";

import {
  describeChip,
  FIELD_LABEL,
  FIELD_OPS,
  newChipId,
  OP_LABEL,
  type FilterChip,
  type FilterField,
  type FilterOp,
} from "./filter-model";

export interface FilterSources {
  users: Array<{ id: string; name: string }>;
  labels: Array<{ id: string; name: string }>;
  projects: Array<{ id: string; name: string }>;
  teams: Array<{ id: string; name: string }>;
}

export interface Option {
  value: string;
  label: string;
}

const FIELDS = Object.keys(FIELD_LABEL) as FilterField[];

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
    default:
      return [];
  }
}

function nameOf(field: FilterField, v: string, s: FilterSources): string {
  return fieldOptions(field, s).find((o) => o.value === v)?.label ?? v;
}

/** Freie Eingabe: Text, Zahl oder Datum (auch relative Dauern wie `-P1W`, dann als Text). */
function inputType(field: FilterField, text: string): "text" | "number" | "date" {
  if (field === "title") return "text";
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

/** Filterleiste (docs/05 §1.6): Chips mit „x“, „Add filter“ öffnet Feldwahl und Wertedialog. */
export function FilterBar({
  chips,
  onChange,
  sources,
  advanced = [],
  onAdvancedChange,
}: {
  chips: FilterChip[];
  onChange: (chips: FilterChip[]) => void;
  sources: FilterSources;
  /** Nicht als Chip darstellbare Filterteile („erweiterter Filter“); bleiben unverändert aktiv. */
  advanced?: Array<Record<string, unknown>>;
  onAdvancedChange?: (rest: Array<Record<string, unknown>>) => void;
}) {
  const [adding, setAdding] = useState<FilterField | null>(null);
  const [editing, setEditing] = useState<FilterChip | null>(null);
  return (
    <div role="group" aria-label="Filters" className="flex flex-wrap items-center gap-1">
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
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" aria-label="Add filter">
            {chips.length === 0 ? <FilterIcon /> : <Plus />} Filter
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className="max-h-[var(--radix-dropdown-menu-content-available-height)] overflow-y-auto"
        >
          {FIELDS.map((f) => (
            <DropdownMenuItem key={f} onSelect={() => setAdding(f)}>
              {FIELD_LABEL[f]}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      {(chips.length > 0 || advanced.length > 0) && (
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
      {adding && (
        <AddFilterDialog
          field={adding}
          sources={sources}
          onClose={() => setAdding(null)}
          onAdd={(c) => onChange([...chips, c])}
        />
      )}
    </div>
  );
}
