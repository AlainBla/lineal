import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/cn";

/** `#RRGGBB`, wie `SAFE_COLOR` in `features/issues/icons.tsx`. */
export const COLOR_RE = /^#[0-9a-f]{6}$/i;

/**
 * Gängige Statusfarben als Palette (LIN-73). Grau, Hellgrau, Gelb, Indigo und Schiefer stammen aus
 * den Standard-Workflow-Farben des Backends (`crates/lineal-app/src/usecases/state.rs`); Indigo
 * `#5e6ad2` ist zugleich die Standardfarbe neuer Status, Orange die Triage-Farbe aus `TYPE_COLOR`
 * (`features/issues/issue-refs.ts`). Türkis, Grün und Rot ergänzen gängige Ampel-/Review-Farben.
 * Freie Farben bleiben über das Hex-Feld bzw. den nativen Farbwähler möglich.
 */
export const STATUS_COLORS: ReadonlyArray<{ hex: string; name: string }> = [
  { hex: "#bec2c8", name: "Gray" },
  { hex: "#95a2b3", name: "Slate" },
  { hex: "#5e6ad2", name: "Indigo" },
  { hex: "#26b5ce", name: "Teal" },
  { hex: "#4cb782", name: "Green" },
  { hex: "#f2c94c", name: "Yellow" },
  { hex: "#fc7840", name: "Orange" },
  { hex: "#eb5757", name: "Red" },
];

/**
 * Farbwähler mit Palette, freier Wahl (Hex-Feld + nativer Farbwähler) und Live-Vorschau statt
 * roher Hex-Eingabe (LIN-73). `onChange` schreibt die Farbe fest (z. B. persistieren), `onPreview`
 * meldet die laufende Auswahl für eine Vorschau (`null` = zurück zur festgeschriebenen Farbe).
 * Festgeschrieben wird nur bewusst (Palette-Klick oder „Apply"), nie bei jedem Eingabe-Event.
 */
export function ColorPicker({
  value,
  onChange,
  onPreview,
  ariaLabel = "Choose color",
}: {
  value: string;
  onChange: (hex: string) => void;
  onPreview?: (hex: string | null) => void;
  ariaLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(value);
  const valid = COLOR_RE.test(draft);
  const normalized = valid ? draft.toLowerCase() : null;

  const preview = (hex: string | null) => onPreview?.(hex);

  const edit = (hex: string) => {
    setDraft(hex);
    if (COLOR_RE.test(hex)) preview(hex.toLowerCase());
  };

  const choose = (hex: string) => {
    const v = hex.toLowerCase();
    setDraft(v);
    preview(null);
    onChange(v);
    setOpen(false);
  };

  const apply = () => {
    if (normalized && normalized !== value.toLowerCase()) {
      choose(normalized);
    } else {
      preview(null);
      setOpen(false);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (o) setDraft(value);
        else preview(null); // Abbruch (Escape/Klick daneben): Vorschau verwerfen.
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel}
          className="flex size-6 shrink-0 items-center justify-center rounded-md border border-input"
        >
          <span
            aria-hidden
            className="size-4 rounded-[3px]"
            style={{ backgroundColor: COLOR_RE.test(value) ? value : "transparent" }}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56">
        <div role="group" aria-label="Color palette" className="grid grid-cols-8 gap-1.5">
          {STATUS_COLORS.map((c) => {
            const selected = (normalized ?? value.toLowerCase()) === c.hex;
            return (
              <button
                key={c.hex}
                type="button"
                aria-label={c.name}
                aria-pressed={selected}
                onClick={() => choose(c.hex)}
                className={cn(
                  "size-5 rounded-full ring-offset-2 ring-offset-popover outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  selected && "ring-2 ring-ring",
                )}
                style={{ backgroundColor: c.hex }}
              />
            );
          })}
        </div>
        <div className="mt-3 flex items-center gap-2">
          <input
            type="color"
            aria-label="Custom color"
            value={valid ? draft : "#000000"}
            onChange={(e) => edit(e.target.value)}
            className="size-8 shrink-0 cursor-pointer rounded-md border border-input bg-transparent p-0.5"
          />
          <Input
            aria-label="Hex color"
            value={draft}
            spellCheck={false}
            onChange={(e) => edit(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                apply();
              }
            }}
            placeholder="#5e6ad2"
            className="h-8 font-mono"
          />
          <Button type="button" size="sm" disabled={!valid} onClick={apply}>
            Apply
          </Button>
        </div>
        {!valid && (
          <p role="alert" className="mt-1 text-xs text-destructive">
            Use the form #RRGGBB.
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
