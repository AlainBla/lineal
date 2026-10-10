import { Link2 } from "lucide-react";
import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { StatusIcon } from "./icons";
import { RELATION_CHOICES, relationInput, type RelationChoice } from "./model";
import { createRelation, fetchIssue } from "./queries";
import { errorMessage } from "./toast-store";

interface AddedRelation {
  choice: RelationChoice;
  issue: Issue;
}

/**
 * Relationen („Blocked by“, „Blocking“, „Related“, „Duplicate of“). Das Schema bietet nur
 * `issueRelationCreate`, keinen Lesezugriff: angezeigt werden die in dieser Sitzung angelegten Relationen.
 */
export function RelationsSection({
  issue,
  dialog,
  onDialog,
}: {
  issue: Issue;
  dialog: RelationChoice | null;
  onDialog: (c: RelationChoice | null) => void;
}) {
  const [added, setAdded] = useState<AddedRelation[]>([]);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useShortcutContext("modal", dialog !== null);
  const choice = RELATION_CHOICES.find((c) => c.id === dialog);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!dialog) return;
    const ref = value.trim();
    if (!ref) {
      setError("Enter an issue identifier such as ENG-12.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const other = await fetchIssue(ref);
      if (other.id === issue.id) {
        setError("An issue cannot be related to itself.");
        return;
      }
      await createRelation(relationInput(dialog, issue.id, other.id));
      setAdded((prev) => [...prev, { choice: dialog, issue: other }]);
      setValue("");
      onDialog(null);
    } catch (err) {
      setError(errorMessage(err, "Could not add the relation."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section aria-label="Related issues" className="flex flex-col gap-1.5">
      <header className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Related</h2>
        <Button variant="ghost" size="sm" className="ml-auto" onClick={() => onDialog("related")}>
          <Link2 /> Add relation
        </Button>
      </header>
      {added.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          Mark issues as blocked by, blocking, related or duplicate (M B, M X, M R, M M).
        </p>
      ) : (
        <ul className="flex flex-col">
          {added.map((r) => (
            <li key={`${r.choice}:${r.issue.id}`} className="flex items-center gap-2 text-sm">
              <span className="w-24 text-xs text-muted-foreground">
                {RELATION_CHOICES.find((c) => c.id === r.choice)?.label}
              </span>
              <StatusIcon type={r.issue.stateType} title={r.issue.stateName} />
              <span className="text-xs text-muted-foreground">{r.issue.identifier}</span>
              <span className="truncate">{r.issue.title}</span>
            </li>
          ))}
        </ul>
      )}
      <Dialog open={dialog !== null} onOpenChange={(o) => !o && onDialog(null)}>
        <DialogContent className="max-w-sm">
          <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-3 p-4">
            <DialogTitle className="text-base font-semibold">
              {choice?.label ?? "Relation"} {issue.identifier}
            </DialogTitle>
            <DialogDescription className="sr-only">
              Enter the identifier of the other issue.
            </DialogDescription>
            <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Relation type">
              {RELATION_CHOICES.map((c) => (
                <Button
                  key={c.id}
                  type="button"
                  size="sm"
                  variant={c.id === dialog ? "secondary" : "ghost"}
                  role="radio"
                  aria-checked={c.id === dialog}
                  onClick={() => onDialog(c.id)}
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
              <Button type="button" variant="ghost" onClick={() => onDialog(null)}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy}>
                Add relation
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </section>
  );
}
