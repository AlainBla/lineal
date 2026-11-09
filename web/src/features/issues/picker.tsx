import { Check } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

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

export interface PickerBodyProps {
  /** Platzhalter und (im Dialog) Titel, z. B. „Change status…“. */
  title: string;
  options: PickerOption[];
  /** Aktueller Wert (Häkchen); bei `multi` mehrere. */
  selected?: string | string[] | null;
  /** Mehrfachauswahl: jede Auswahl schaltet um, der Picker bleibt offen. */
  multi?: boolean;
  onSelect: (id: string) => void;
  emptyText?: string;
  /** Beim Mounten das Suchfeld fokussieren (für das Flyout-Submenü, LIN-53). */
  autoFocus?: boolean;
  /** Nach einer Einzelauswahl aufrufen (Dialog schließen bzw. Flyout/Menü schließen). */
  onDone?: () => void;
}

interface PropertyPickerProps extends Omit<PickerBodyProps, "autoFocus" | "onDone"> {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * Generischer Picker (docs/08 §1.4): Popover-artiger Dialog mit Suchfeld, aktueller Wert mit Häkchen,
 * Ziffern 1–9 wählen die n-te Option (solange das Suchfeld leer ist), `Esc` schließt.
 * Während er offen ist, gilt der Shortcut-Kontext `modal`.
 */
export function PropertyPicker({ open, onOpenChange, ...body }: PropertyPickerProps) {
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent position="top" className="max-w-sm" aria-describedby={undefined}>
        <DialogTitle className="sr-only">{body.title}</DialogTitle>
        <DialogDescription className="sr-only">
          Type to filter, press a number to choose, Enter to confirm, Escape to close.
        </DialogDescription>
        {/* Der Inhalt hängt nur im offenen Zustand: Suchtext beginnt bei jedem Öffnen leer. */}
        <PickerBody {...body} autoFocus onDone={() => onOpenChange(false)} />
      </DialogContent>
    </Dialog>
  );
}

/**
 * Such-/Options-Inhalt ohne Hülle: steckt im Dialog (`PropertyPicker`) und im Hover-Flyout des
 * Kontextmenüs (`PropertySubmenu`, LIN-53). `onDone` schließt die jeweilige Hülle nach Einzelwahl.
 */
export function PickerBody({
  title,
  options,
  selected,
  multi = false,
  onSelect,
  emptyText = "No results.",
  autoFocus = false,
  onDone,
}: PickerBodyProps) {
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!autoFocus) return;
    // Nach Radix' Fokus-Scope fokussieren (beim Öffnen per Tastatur fokussiert Radix sonst den
    // Submenü-Container statt des Suchfelds); `0ms` reicht, FocusScope hält den Fokus dann im Flyout.
    const id = setTimeout(() => rootRef.current?.querySelector("input")?.focus(), 0);
    return () => clearTimeout(id);
  }, [autoFocus]);
  const chosen = new Set(Array.isArray(selected) ? selected : selected ? [selected] : []);

  const pick = (id: string) => {
    onSelect(id);
    if (!multi) onDone?.();
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
    <div ref={rootRef} className="contents">
      <Command loop label={title}>
        <CommandInput
          placeholder={title}
          value={query}
          onValueChange={setQuery}
          onKeyDown={onKeyDown}
        />
        <CommandList
          // Klick auf eine Option darf den Fokus nicht vom Suchfeld nehmen: sonst schließt ein
          // umschließendes Radix-Submenü (Flyout, LIN-53) beim pointerdown, bevor der Klick wählt.
          onPointerDown={(e) => e.preventDefault()}
          onMouseDown={(e) => e.preventDefault()}
        >
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
    </div>
  );
}
