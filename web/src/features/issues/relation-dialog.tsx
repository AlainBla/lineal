import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { RELATION_CHOICES, relationInput, type RelationChoice } from "./model";
import { createRelation, fetchIssue, keys } from "./queries";
import { errorMessage } from "./toast-store";

/**
 * Eigenständiger Relationen-Dialog: Zielticket per Kennung wählen und für jedes der `issues` eine
 * Relation anlegen (Selbstbezüge werden übersprungen). „Duplicate of“ gibt es nur für ein einzelnes
 * Ticket. Genutzt von der Ticketseite (`RelationsSection`) und vom Kontextmenü.
 */
export function RelationDialog({
  issues,
  choice,
  onChoice,
  onClose,
}: {
  issues: Issue[];
  /** `null` = geschlossen. */
  choice: RelationChoice | null;
  onChoice: (c: RelationChoice) => void;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useShortcutContext("modal", choice !== null);
  const multi = issues.length > 1;
  const current = RELATION_CHOICES.find((c) => c.id === choice);
  const choices = RELATION_CHOICES.filter((c) => !multi || c.id !== "duplicate");
  const subject = multi ? `${issues.length} issues` : (issues[0]?.identifier ?? "");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!choice) return;
    const ref = value.trim();
    if (!ref) {
      setError("Enter an issue identifier such as ENG-12.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const other = await fetchIssue(ref);
      const sources = issues.filter((i) => i.id !== other.id);
      if (sources.length === 0) {
        setError("An issue cannot be related to itself.");
        return;
      }
      for (const src of sources) await createRelation(relationInput(choice, src.id, other.id));
      await qc.invalidateQueries({ queryKey: keys.issueDetails });
      setValue("");
      onClose();
    } catch (err) {
      setError(errorMessage(err, "Could not add the relation."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={choice !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">
            {current?.label ?? "Relation"} {subject}
          </DialogTitle>
          <DialogDescription className="sr-only">
            Enter the identifier of the other issue.
          </DialogDescription>
          <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Relation type">
            {choices.map((c) => (
              <Button
                key={c.id}
                type="button"
                size="sm"
                variant={c.id === choice ? "secondary" : "ghost"}
                role="radio"
                aria-checked={c.id === choice}
                onClick={() => onChoice(c.id)}
              >
                {c.label}
              </Button>
            ))}
          </div>
          <Input
            autoFocus
            aria-label="Issue identifier"
            placeholder="Issue identifier, e.g. ENG-12"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            aria-invalid={error !== null}
          />
          {error && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              Add relation
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
