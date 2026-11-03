import { useQuery } from "@tanstack/react-query";
import { useEffect, useState, type KeyboardEvent } from "react";

import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import { cn } from "@/lib/utils";

import { StatusIcon } from "./icons";

export type IssueCandidate = Pick<Issue, "id" | "identifier" | "title" | "stateType">;

async function searchCandidates(term: string, signal?: AbortSignal): Promise<IssueCandidate[]> {
  const res = await gqlRequest<
    { searchIssues: { nodes: Array<{ issue: IssueCandidate }> } },
    { term: string; first: number }
  >(
    `query IssuePickerSearch($term: String!, $first: Int) {
      searchIssues(term: $term, first: $first) { nodes { issue { id identifier title stateType } } }
    }`,
    { term, first: 10 },
    { signal },
  );
  return res.searchIssues.nodes.map((n) => n.issue);
}

/**
 * Ticketsuche (Kennung oder Titel) mit Tastaturbedienung: ↑/↓ wählen, Enter übernimmt. `exclude`
 * blendet die genannten Issue-IDs aus den Treffern aus.
 */
export function IssuePicker({
  exclude,
  disabled,
  invalid,
  onPick,
}: {
  exclude: string[];
  disabled?: boolean;
  invalid?: boolean;
  onPick: (issue: IssueCandidate) => void;
}) {
  const [term, setTerm] = useState("");
  const [debounced, setDebounced] = useState("");
  const [active, setActive] = useState(0);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(term.trim()), 200);
    return () => clearTimeout(t);
  }, [term]);
  const results = useQuery({
    queryKey: ["issues", "picker-search", debounced],
    enabled: debounced !== "",
    queryFn: ({ signal }) => searchCandidates(debounced, signal),
  });
  const candidates = (results.data ?? []).filter((c) => !exclude.includes(c.id));
  const index = Math.min(active, Math.max(candidates.length - 1, 0));
  const activeId = candidates[index] ? `issue-picker-${candidates[index].id}` : undefined;

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (candidates.length === 0) return;
      const d = e.key === "ArrowDown" ? 1 : -1;
      setActive((index + d + candidates.length) % candidates.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const c = candidates[index];
      if (c && !disabled) onPick(c);
    }
  };

  return (
    <div className="flex flex-col gap-1">
      <Input
        autoFocus
        role="combobox"
        aria-expanded={debounced !== ""}
        aria-controls="issue-picker-list"
        aria-activedescendant={activeId}
        aria-autocomplete="list"
        aria-label="Search issues"
        placeholder="Search by title or identifier…"
        value={term}
        onChange={(e) => {
          setTerm(e.target.value);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
        aria-invalid={invalid}
      />
      {debounced !== "" && (
        <ul
          id="issue-picker-list"
          role="listbox"
          aria-label="Issue results"
          className="flex flex-col"
        >
          {results.isLoading && (
            <li role="presentation" className="px-2 py-1 text-xs text-muted-foreground">
              Searching…
            </li>
          )}
          {!results.isLoading && candidates.length === 0 && (
            <li role="presentation" className="px-2 py-1 text-xs text-muted-foreground">
              No results.
            </li>
          )}
          {candidates.map((c, i) => (
            <li
              key={c.id}
              id={`issue-picker-${c.id}`}
              role="option"
              aria-selected={i === index}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-surface-hover",
                i === index && "bg-surface-hover",
              )}
              onMouseEnter={() => setActive(i)}
              onClick={() => !disabled && onPick(c)}
            >
              <StatusIcon type={c.stateType} />
              <span className="text-xs text-muted-foreground tabular-nums">{c.identifier}</span>
              <span className="truncate">{c.title}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
