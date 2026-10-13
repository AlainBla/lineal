/**
 * Kleine Bearbeitungsfelder der Planung (Projekt-/Initiativ-Detail, Anlegen-Dialoge): sie halten nur
 * den Entwurf; Speichern, Rechte und Validierung liegen beim Server, dessen Fehler die Aufrufer zeigen.
 */
import { X } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { selectClass } from "@/features/settings/ui";

export interface Option {
  value: string;
  label: string;
}

export const DATE_RESOLUTIONS: Option[] = [
  { value: "", label: "Day" },
  { value: "month", label: "Month" },
  { value: "quarter", label: "Quarter" },
  { value: "halfYear", label: "Half year" },
  { value: "year", label: "Year" },
];

export const PROJECT_STATUS_TYPES: Option[] = [
  { value: "backlog", label: "Backlog" },
  { value: "planned", label: "Planned" },
  { value: "started", label: "In progress" },
  { value: "paused", label: "Paused" },
  { value: "completed", label: "Completed" },
  { value: "canceled", label: "Canceled" },
];

export const INITIATIVE_STATUSES: Option[] = [
  { value: "proposed", label: "Proposed" },
  { value: "planned", label: "Planned" },
  { value: "active", label: "Active" },
  { value: "completed", label: "Completed" },
  { value: "canceled", label: "Canceled" },
];

export function FieldBox({
  label,
  children,
  className,
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={className ?? "flex flex-col gap-1"}>
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

/** Einzeiliges Textfeld: speichert bei Enter/Verlassen, wenn sich der Wert geändert hat. */
export function TextField({
  label,
  value,
  onCommit,
  maxLength,
  required,
  placeholder,
}: {
  label: string;
  value: string;
  onCommit: (v: string) => void;
  maxLength?: number;
  required?: boolean;
  placeholder?: string;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  // Neuer Wert von außen (z. B. nach dem Speichern): Entwurf nachziehen, sofern nicht gerade editiert.
  if (value !== seen) {
    setSeen(value);
    setDraft(value);
  }
  const commit = () => {
    const next = draft.trim();
    if (next === value) return;
    if (required && next === "") {
      setDraft(value);
      return;
    }
    onCommit(next);
  };
  return (
    <FieldBox label={label}>
      <Input
        aria-label={label}
        value={draft}
        maxLength={maxLength}
        placeholder={placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
    </FieldBox>
  );
}

export function SelectField({
  label,
  value,
  options,
  onChange,
  emptyLabel,
}: {
  label: string;
  value: string;
  options: Option[];
  onChange: (v: string) => void;
  /** Eintrag für „nichts“ (Wert `""`), z. B. „No lead“. */
  emptyLabel?: string;
}) {
  return (
    <FieldBox label={label}>
      <select
        aria-label={label}
        className={selectClass}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {emptyLabel !== undefined && <option value="">{emptyLabel}</option>}
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </FieldBox>
  );
}

/** Datum mit Auflösung (Tag, Monat, Quartal, Halbjahr, Jahr); leer = kein Datum. */
export function DateField({
  label,
  date,
  resolution,
  onChange,
  dayOnly,
}: {
  label: string;
  date: string | null | undefined;
  resolution?: string | null;
  onChange: (date: string | null, resolution: string | null) => void;
  dayOnly?: boolean;
}) {
  const day = date ? date.slice(0, 10) : "";
  return (
    <FieldBox label={label}>
      <div className="flex items-center gap-2">
        <Input
          aria-label={label}
          type="date"
          className="w-40"
          value={day}
          onChange={(e) =>
            onChange(e.target.value || null, e.target.value ? resolution || null : null)
          }
        />
        {!dayOnly && day && (
          <select
            aria-label={`${label} resolution`}
            className={selectClass}
            value={resolution ?? ""}
            onChange={(e) => onChange(day, e.target.value || null)}
          >
            {DATE_RESOLUTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        )}
      </div>
    </FieldBox>
  );
}

/** Liste von Verweisen (Teams, Labels, Mitglieder …) mit Entfernen und Hinzufügen. */
export function RefList({
  label,
  items,
  choices,
  onAdd,
  onRemove,
  addLabel,
  busy,
}: {
  label: string;
  items: Array<{ id: string; name: string }>;
  /** Auswahl zum Hinzufügen (bereits enthaltene werden ausgeblendet). */
  choices: Option[];
  onAdd: (id: string) => void;
  onRemove: (id: string) => void;
  addLabel?: string;
  busy?: boolean;
}) {
  const present = new Set(items.map((i) => i.id));
  const rest = choices.filter((c) => !present.has(c.value));
  return (
    <FieldBox label={label}>
      <ul aria-label={label} className="flex flex-wrap items-center gap-1">
        {items.length === 0 && <li className="text-sm text-muted-foreground">None</li>}
        {items.map((i) => (
          <li
            key={i.id}
            className="inline-flex items-center gap-1 rounded-full border border-border px-2 py-0.5 text-xs"
          >
            {i.name}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-4"
              disabled={busy}
              aria-label={`Remove ${i.name} from ${label.toLowerCase()}`}
              onClick={() => onRemove(i.id)}
            >
              <X className="size-3" />
            </Button>
          </li>
        ))}
        <li>
          <select
            aria-label={addLabel ?? `Add to ${label.toLowerCase()}`}
            className={selectClass}
            value=""
            disabled={busy || rest.length === 0}
            onChange={(e) => e.target.value && onAdd(e.target.value)}
          >
            <option value="">{addLabel ?? "Add…"}</option>
            {rest.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </li>
      </ul>
    </FieldBox>
  );
}
