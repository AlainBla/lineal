import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useShortcutContext } from "@/lib/shortcuts";

/** Fälligkeitsdatum setzen oder löschen (Detailseite und Listen-Kürzel ⇧D). */
export function DueDateDialog({
  open,
  current,
  onClose,
  onSave,
}: {
  open: boolean;
  current: string | null;
  onClose: () => void;
  onSave: (d: string | null) => void;
}) {
  const [value, setValue] = useState(current ?? "");
  useShortcutContext("modal", open);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xs">
        <form
          className="flex flex-col gap-3 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            // Portal-Ereignisse bubblen in React bis zum umgebenden Formular (Create-Modal).
            e.stopPropagation();
            onSave(value || null);
            onClose();
          }}
        >
          <DialogTitle className="text-base font-semibold">Set due date</DialogTitle>
          <DialogDescription className="sr-only">Pick a date or clear it.</DialogDescription>
          <Input
            type="date"
            autoFocus
            aria-label="Due date"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <div className="flex justify-between">
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                onSave(null);
                onClose();
              }}
            >
              Clear
            </Button>
            <Button type="submit">Save</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
