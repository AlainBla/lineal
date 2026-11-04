import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import type { Issue } from "@/gql/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { useIssue, useIssueExtras } from "./hooks";
import type { IssueActions } from "./issue-actions";
import { IssuePicker } from "./issue-picker";

/**
 * Suchdialog für ein Parent-Issue: wählen oder (bei vorhandenem Parent) entfernen. `exclude`
 * blendet die genannten Issues (ID oder Liste) aus; die Suche liefert `IssuePicker`. Zyklen und
 * Selbstbezug lehnt der Server beim Speichern ab.
 */
export function ParentSearchDialog({
  currentParentId,
  exclude,
  title = "Set parent issue",
  description,
  onChoose,
  onClose,
}: {
  currentParentId: string | null;
  exclude?: string | string[];
  title?: string;
  description?: string;
  onChoose: (parentId: string | null) => void;
  onClose: () => void;
}) {
  useShortcutContext("modal");
  const current = useIssue(currentParentId ?? "");
  const excluded = exclude === undefined ? [] : Array.isArray(exclude) ? exclude : [exclude];

  const choose = (parentId: string | null) => {
    onChoose(parentId);
    onClose();
  };

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent position="top" className="max-w-md">
        <div className="flex flex-col gap-3 p-4">
          <DialogTitle className="text-base font-semibold">{title}</DialogTitle>
          <DialogDescription className="sr-only">
            {description ?? "Search for the issue that becomes the parent."}
          </DialogDescription>
          {currentParentId && (
            <div className="flex items-center gap-2 text-sm">
              <span className="text-muted-foreground">Current parent:</span>
              <span>{current.data ? `${current.data.identifier} ${current.data.title}` : "…"}</span>
              <Button variant="ghost" size="sm" className="ml-auto" onClick={() => choose(null)}>
                Remove parent
              </Button>
            </div>
          )}
          <IssuePicker exclude={excluded} onPick={(c) => choose(c.id)} />
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** „Set parent issue…“ (Strg ⇧ P, docs/08): Parent des Issues setzen, wechseln oder entfernen. */
export function ParentDialog({
  issue,
  actions,
  onClose,
  alsoExclude = [],
}: {
  issue: Issue;
  actions: IssueActions;
  onClose: () => void;
  /** Weitere Issues einer Mehrfachauswahl, die ebenfalls nicht Parent werden dürfen. */
  alsoExclude?: string[];
}) {
  // Direkte Kinder ausblenden (Zyklen); tiefere Nachfahren lehnt der Server beim Speichern ab.
  const extras = useIssueExtras(issue.id);
  const exclude = [issue.id, ...alsoExclude, ...(extras.data?.children.map((c) => c.id) ?? [])];
  return (
    <ParentSearchDialog
      currentParentId={issue.parentId}
      exclude={exclude}
      description={`Search for the issue that becomes the parent of ${issue.identifier}.`}
      onChoose={(id) => actions.setParent(issue, id)}
      onClose={onClose}
    />
  );
}
