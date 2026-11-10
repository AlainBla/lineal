import { Link } from "@tanstack/react-router";
import { Link2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import type { Issue } from "@/gql/graphql";

import { useIssueExtras } from "./hooks";
import { StatusIcon } from "./icons";
import { issuePath, relationGroups, type RelationChoice } from "./model";
import { RelationDialog } from "./relation-dialog";

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
  const extras = useIssueExtras(issue.id);
  const groups = extras.data ? relationGroups(extras.data.relations) : [];

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
                  <span className="w-24 shrink-0 text-xs whitespace-nowrap text-muted-foreground">
                    {g.label}
                  </span>
                  <StatusIcon type={r.stateType} color={r.stateColor} title={r.stateName} />
                  <span className="shrink-0 text-xs whitespace-nowrap text-muted-foreground">
                    {r.identifier}
                  </span>
                  <span className="truncate">{r.title}</span>
                </Link>
              </li>
            )),
          )}
        </ul>
      )}
      <RelationDialog
        issues={[issue]}
        choice={dialog}
        onChoice={onDialog}
        onClose={() => onDialog(null)}
      />
    </section>
  );
}
