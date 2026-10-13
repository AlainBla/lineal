import type { IssueLabel } from "@/gql/graphql";

import { PRIORITIES } from "./model";
import type { FormAnswerValue, FormField } from "./templates";

const CONTROL =
  "rounded-md border border-border-strong bg-transparent px-2 py-1.5 text-sm outline-none focus-visible:border-ring";

/**
 * Felder eines Form-Templates (docs/06 §2.8): Text, Langtext, Dropdown, Checkboxen, Datum,
 * Hinweistext sowie die Property-Felder Titel, Priority, Fälligkeit und Label-Gruppe.
 */
export function TemplateForm({
  fields,
  answers,
  onChange,
  labels,
  invalid,
}: {
  fields: FormField[];
  answers: Record<string, FormAnswerValue>;
  onChange: (id: string, value: FormAnswerValue) => void;
  labels: IssueLabel[];
  /** Felder, die nach einem Absendeversuch fehlen. */
  invalid: string[];
}) {
  const text = (id: string) => (typeof answers[id] === "string" ? (answers[id] as string) : "");
  return (
    <div role="group" aria-label="Form" className="flex flex-col gap-3 px-4 pt-3">
      {fields.map((f) => {
        const missing = invalid.includes(f.id);
        const label = (
          <span className="text-xs font-medium text-muted-foreground">
            {f.label}
            {f.required && <span aria-hidden> *</span>}
          </span>
        );
        const common = {
          "aria-label": f.label,
          "aria-required": f.required || undefined,
          "aria-invalid": missing || undefined,
        };
        switch (f.kind) {
          case "instructions":
            return (
              <p key={f.id} className="text-sm text-muted-foreground">
                {f.text ?? f.label}
              </p>
            );
          case "checkboxes": {
            const value = Array.isArray(answers[f.id]) ? (answers[f.id] as string[]) : [];
            return (
              <fieldset key={f.id} className="flex flex-col gap-1" aria-label={f.label}>
                <legend>{label}</legend>
                {f.options.map((o) => (
                  <label key={o} className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={value.includes(o)}
                      onChange={(e) =>
                        onChange(
                          f.id,
                          e.target.checked ? [...value, o] : value.filter((x) => x !== o),
                        )
                      }
                    />
                    {o}
                  </label>
                ))}
                {missing && <span className="text-xs text-destructive">Required</span>}
              </fieldset>
            );
          }
          default:
            break;
        }
        return (
          <label key={f.id} className="flex flex-col gap-1">
            {label}
            {f.kind === "longText" ? (
              <textarea
                {...common}
                rows={3}
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              />
            ) : f.kind === "dropdown" ? (
              <select
                {...common}
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              >
                <option value="">Select…</option>
                {f.options.map((o) => (
                  <option key={o} value={o}>
                    {o}
                  </option>
                ))}
              </select>
            ) : f.kind === "priority" ? (
              <select
                {...common}
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              >
                <option value="">Select…</option>
                {PRIORITIES.map((p) => (
                  <option key={p.value} value={String(p.value)}>
                    {p.label}
                  </option>
                ))}
              </select>
            ) : f.kind === "labelGroup" ? (
              <select
                {...common}
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              >
                <option value="">Select…</option>
                {labels
                  .filter((l) => !l.isGroup && l.parentId === f.labelGroupId)
                  .map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.name}
                    </option>
                  ))}
              </select>
            ) : f.kind === "date" || f.kind === "dueDate" ? (
              <input
                {...common}
                type="date"
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              />
            ) : (
              <input
                {...common}
                type="text"
                className={CONTROL}
                value={text(f.id)}
                onChange={(e) => onChange(f.id, e.target.value)}
              />
            )}
            {missing && <span className="text-xs text-destructive">Required</span>}
          </label>
        );
      })}
    </div>
  );
}
