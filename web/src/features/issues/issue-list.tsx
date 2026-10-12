import { ChevronDown, Plus } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";

import { StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { IssueRow } from "./issue-row";
import type { BoardColumn } from "./model";
import type { Selection } from "./selection";

interface ListProps {
  groups: BoardColumn[];
  ws: string;
  lookup: IssueLookup;
  focusedId: string | null;
  onFocus: (id: string) => void;
  /** „+“ im Gruppenkopf: Issue mit Gruppenstatus anlegen. */
  onCreateInGroup?: (stateId: string) => void;
  selection?: Selection;
}

/** Liste nach Status gruppiert (einklappbar, `+` je Gruppe). */
export function IssueList({
  groups,
  ws,
  lookup,
  focusedId,
  onFocus,
  onCreateInGroup,
  selection,
}: ListProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  return (
    <div role="list" aria-label="Issues">
      {groups.map(({ state, issues }) => {
        const closed = collapsed.has(state.id);
        return (
          <section key={state.id} aria-label={state.name}>
            <header className="flex h-8 items-center gap-2 border-b border-border bg-surface px-4 text-sm">
              <button
                type="button"
                className="flex items-center gap-2 rounded-sm outline-none focus-visible:outline-2"
                aria-expanded={!closed}
                aria-label={`${closed ? "Expand" : "Collapse"} ${state.name}`}
                onClick={() => toggle(state.id)}
              >
                <ChevronDown
                  className={cn(
                    "size-3.5 text-muted-foreground transition-transform",
                    closed && "-rotate-90",
                  )}
                  aria-hidden
                />
                <StatusIcon type={state.type} color={state.color} />
                <span className="font-medium">{state.name}</span>
                <span className="text-xs text-muted-foreground">{issues.length}</span>
              </button>
              {onCreateInGroup && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="ml-auto size-6"
                  aria-label={`Create issue in ${state.name}`}
                  onClick={() => onCreateInGroup(state.id)}
                >
                  <Plus className="size-3.5" />
                </Button>
              )}
            </header>
            {!closed &&
              issues.map((issue) => (
                <div role="listitem" key={issue.id}>
                  <IssueRow
                    issue={issue}
                    ws={ws}
                    lookup={lookup}
                    focused={focusedId === issue.id}
                    onFocus={onFocus}
                    selection={selection}
                  />
                </div>
              ))}
          </section>
        );
      })}
    </div>
  );
}
