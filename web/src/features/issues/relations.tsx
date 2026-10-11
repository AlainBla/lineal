import { Link } from "@tanstack/react-router";
import { Link2 } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { useIssueExtras } from "./hooks";
import {
  issuePath,
  relationGroups,
  RELATION_CHOICES,
  relationInput,
  type RelationChoice,
} from "./model";
import { createRelation, fetchIssue, keys } from "./queries";
import { errorMessage } from "./toast-store";

/** Relationen („Blocked by“, „Blocking“, „Related“, „Duplicate of“) aus `Issue.relations` der Detail-Query. */
export function RelationsSection({
  issue,
  ws,
  dialog,
  onDialog,
}: {
  issue: Issue;
  ws: string;
  dialog: RelationChoice | null;
  onDialog: (c: RelationChoice | null) => void;
}) {
  const qc = useQueryClient();
  const extras = useIssueExtras(issue.id);
  const groups = extras.data ? relationGroups(extras.data.relations) : [];
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
      await qc.invalidateQueries({ queryKey: keys.issueDetails });
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
      {groups.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          {extras.isError
            ? "Could not load relations."
            : "Mark issues as blocked by, blocking, related or duplicate (M B, M X, M R, M M)."}
        </p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {groups.flatMap((g) =>
            g.items.map((r) => (
              <li key={`${g.label}:${r.id}`}>
                <Link
                  to={issuePath(ws, r.identifier) as "/"}
                  className="flex items-center gap-2 rounded px-1 text-sm hover:bg-surface-hover"
                >
                  <span className="w-24 text-xs text-muted-foreground">{g.label}</span>
                  <span className="text-xs text-muted-foreground">{r.identifier}</span>
                  <span className="truncate">{r.title}</span>
                </Link>
              </li>
            )),
          )}
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
