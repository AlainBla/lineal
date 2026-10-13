import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { useShortcutContext } from "@/lib/shortcuts";

import { StatusIcon } from "./icons";
import { useIssue } from "./hooks";
import type { IssueActions } from "./issue-actions";

type Candidate = Pick<Issue, "id" | "identifier" | "title" | "stateType">;

async function searchParents(term: string, signal?: AbortSignal): Promise<Candidate[]> {
  const res = await gqlRequest<
    { searchIssues: { nodes: Array<{ issue: Candidate }> } },
    { term: string; first: number }
  >(
    `query ParentSearch($term: String!, $first: Int) {
      searchIssues(term: $term, first: $first) { nodes { issue { id identifier title stateType } } }
    }`,
    { term, first: 8 },
    { signal },
  );
  return res.searchIssues.nodes.map((n) => n.issue);
}

/**
 * Suchdialog für ein Parent-Issue: wählen oder (bei vorhandenem Parent) entfernen. `exclude`
 * blendet das Issue selbst aus. Zyklen und Selbstbezug lehnt der Server beim Speichern ab.
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
  exclude?: string;
  title?: string;
  description?: string;
  onChoose: (parentId: string | null) => void;
  onClose: () => void;
}) {
  useShortcutContext("modal");
  const [term, setTerm] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebounced(term.trim()), 200);
    return () => clearTimeout(t);
  }, [term]);
  const current = useIssue(currentParentId ?? "");
  const results = useQuery({
    queryKey: ["issues", "parent-search", debounced],
    enabled: debounced !== "",
    queryFn: ({ signal }) => searchParents(debounced, signal),
  });
  const candidates = (results.data ?? []).filter((c) => c.id !== exclude);

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
          <Input
            autoFocus
            aria-label="Search issues"
            placeholder="Search by title or identifier…"
            value={term}
            onChange={(e) => setTerm(e.target.value)}
          />
          {debounced !== "" && (
            <ul aria-label="Parent candidates" className="flex flex-col">
              {results.isLoading && (
                <li className="px-2 py-1 text-xs text-muted-foreground">Searching…</li>
              )}
              {!results.isLoading && candidates.length === 0 && (
                <li className="px-2 py-1 text-xs text-muted-foreground">No results.</li>
              )}
              {candidates.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-surface-hover"
                    onClick={() => choose(c.id)}
                  >
                    <StatusIcon type={c.stateType} />
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {c.identifier}
                    </span>
                    <span className="truncate">{c.title}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
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
}: {
  issue: Issue;
  actions: IssueActions;
  onClose: () => void;
}) {
  return (
    <ParentSearchDialog
      currentParentId={issue.parentId}
      exclude={issue.id}
      description={`Search for the issue that becomes the parent of ${issue.identifier}.`}
      onChoose={(id) => actions.setParent(issue, id)}
      onClose={onClose}
    />
  );
}
