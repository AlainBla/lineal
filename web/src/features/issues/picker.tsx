import { Check } from "lucide-react";
import { useState, type ReactNode } from "react";

import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useShortcutContext } from "@/lib/shortcuts";

export interface PickerOption {
  id: string;
  label: string;
  icon?: ReactNode;
  /** Zusätzliche Suchbegriffe. */
  keywords?: string[];
  /** Rechtsbündiger Hinweis (z. B. „Current“). */
  hint?: string;
  /**
   * Ziffer, die diese Option direkt wählt. Hat mindestens eine Option eine Ziffer, gilt nur diese
   * Zuordnung (Priority: Ziffer = Prioritätswert); sonst zählt die Position (1–9).
   */
  digit?: string;
}

interface PropertyPickerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Platzhalter und Dialogtitel, z. B. „Change status…“. */
  title: string;
  options: PickerOption[];
  /** Aktueller Wert (Häkchen); bei `multi` mehrere. */
  selected?: string | string[] | null;
  /** Mehrfachauswahl: jede Auswahl schaltet um, der Picker bleibt offen. */
  multi?: boolean;
  onSelect: (id: string) => void;
  emptyText?: string;
}

/**
 * Generischer Picker (docs/08 §1.4): Popover-artiger Dialog mit Suchfeld, aktueller Wert mit Häkchen,
 * Ziffern 1–9 wählen die n-te Option (solange das Suchfeld leer ist), `Esc` schließt.
 * Während er offen ist, gilt der Shortcut-Kontext `modal`.
 */
export function PropertyPicker(props: PropertyPickerProps) {
  const { open, onOpenChange, title } = props;
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent position="top" className="max-w-sm" aria-describedby={undefined}>
        <DialogTitle className="sr-only">{title}</DialogTitle>
        <DialogDescription className="sr-only">
          Type to filter, press a number to choose, Enter to confirm, Escape to close.
        </DialogDescription>
        {/* Der Inhalt hängt nur im offenen Zustand: Suchtext beginnt bei jedem Öffnen leer. */}
        <PickerBody {...props} />
      </DialogContent>
    </Dialog>
  );
}

function PickerBody({
  onOpenChange,
  title,
  options,
  selected,
  multi = false,
  onSelect,
  emptyText = "No results.",
}: PropertyPickerProps) {
  const [query, setQuery] = useState("");
  const chosen = new Set(Array.isArray(selected) ? selected : selected ? [selected] : []);

  const pick = (id: string) => {
    onSelect(id);
    if (!multi) onOpenChange(false);
  };

  const explicitDigits = options.some((o) => o.digit !== undefined);
  const digitOf = (o: PickerOption, i: number): string | undefined =>
    explicitDigits ? o.digit : i < 9 ? String(i + 1) : undefined;

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (query !== "" || e.ctrlKey || e.metaKey || e.altKey) return;
    if (/^[0-9]$/.test(e.key)) {
      const opt = options.find((o, i) => digitOf(o, i) === e.key);
      if (opt) {
        e.preventDefault();
        pick(opt.id);
      }
    }
  };

  return (
    <Command loop label={title}>
      <CommandInput
        placeholder={title}
        value={query}
        onValueChange={setQuery}
        onKeyDown={onKeyDown}
      />
      <CommandList>
        <CommandEmpty>{emptyText}</CommandEmpty>
        {options.map((o, i) => (
          <CommandItem
            key={o.id}
            value={`${o.label} ${o.id}`}
            keywords={o.keywords}
            onSelect={() => pick(o.id)}
            data-picker-id={o.id}
          >
            {o.icon}
            <span className="truncate">{o.label}</span>
            <span className="ml-auto flex items-center gap-2 text-xs text-subtle-foreground">
              {o.hint}
              {chosen.has(o.id) && <Check aria-label="Selected" className="size-3.5" />}
              {query === "" && digitOf(o, i) !== undefined && (
                <span aria-hidden>{digitOf(o, i)}</span>
              )}
            </span>
          </CommandItem>
        ))}
      </CommandList>
    </Command>
  );
}
