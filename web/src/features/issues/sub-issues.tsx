import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { Issue } from "@/gql/graphql";
import { useSingleFlight } from "@/lib/use-single-flight";

import { useCreateIssue, useIssueExtras } from "./hooks";
import { Avatar, StatusIcon } from "./icons";
import type { IssueLookup } from "./issue-lookup";
import { issuePath, subIssueProgress } from "./model";
import { errorMessage, showToast } from "./toast-store";

/**
 * Sub-Issues eines Issues (docs/08 §2.2, F8). Die Kinder kommen aus `Issue.children` der Detail-Query
 * (teamübergreifend, nur für den Aufrufer sichtbare); Anlegen invalidiert diese Query.
 */
export function SubIssuesSection({
  issue,
  ws,
  lookup,
  adding,
  onAddingChange,
}: {
  issue: Issue;
  ws: string;
  lookup: IssueLookup;
  adding: boolean;
  onAddingChange: (v: boolean) => void;
}) {
  const extras = useIssueExtras(issue.id);
  const create = useCreateIssue();
  const [title, setTitle] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const children = (extras.data?.children ?? []).filter((c) => !c.trashed);
  const progress = subIssueProgress(children);

  const singleFlight = useSingleFlight();
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const t = title.trim();
    if (!t) return;
    // Ref-Sperre: mehrfaches Enter legt nur ein Sub-Issue an (B4.13).
    void singleFlight(async () => {
      try {
        await create.mutateAsync({ teamId: issue.teamId, title: t, parentId: issue.id });
        setTitle("");
        inputRef.current?.focus();
      } catch (err) {
        showToast({ message: errorMessage(err, "Could not create sub-issue."), tone: "error" });
      }
    });
  };

  return (
    <section aria-label="Sub-issues" className="flex flex-col gap-1.5">
      <header className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Sub-issues</h2>
        {progress.total > 0 && (
          <span className="text-xs text-muted-foreground" aria-label="Sub-issue progress">
            {progress.done}/{progress.total}
          </span>
        )}
        <Button variant="ghost" size="sm" className="ml-auto" onClick={() => onAddingChange(true)}>
          <Plus /> {children.length === 0 ? "Add sub-issues" : "New sub-issue"}
        </Button>
      </header>
      {extras.isError && (
        <p role="alert" className="text-xs text-destructive">
          Could not load sub-issues.
        </p>
      )}
      {children.length > 0 && (
        <ul className="flex flex-col rounded-lg border border-border">
          {children.map((c) => {
            const assignee = lookup.assigneeName(c.assigneeId);
            return (
              <li key={c.id} className="border-b border-border last:border-b-0">
                <Link
                  to={issuePath(ws, c.identifier) as "/"}
                  className="flex h-8 items-center gap-2 px-3 text-sm hover:bg-surface-hover"
                >
                  <StatusIcon
                    type={c.stateType}
                    color={lookup.state(c.stateId)?.color}
                    title={c.stateName}
                  />
                  <span className="w-16 text-xs text-muted-foreground">{c.identifier}</span>
                  <span className="min-w-0 flex-1 truncate">{c.title}</span>
                  {assignee && <Avatar name={assignee} />}
                </Link>
              </li>
            );
          })}
        </ul>
      )}
      {adding && (
        <form onSubmit={submit} className="flex gap-2">
          <Input
            ref={inputRef}
            autoFocus
            aria-label="Sub-issue title"
            placeholder="Sub-issue title, press Enter to add"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onAddingChange(false);
            }}
          />
        </form>
      )}
    </section>
  );
}
