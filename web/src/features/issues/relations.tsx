import { Link } from "@tanstack/react-router";
import { Link2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";
import type { Issue } from "@/gql/graphql";

import { useIssueExtras } from "./hooks";
import { BlockedIcon, StatusIcon } from "./icons";
import { isOpenType, issuePath, relationGroups, type RelationChoice } from "./model";
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
          {groups.flatMap((g) => {
            // „Blocked by"/„Blocking" sichtbar hervorheben (LIN-79): Blocked-Icon + Warnfarbe, solange
            // das Gegenüber offen ist; erledigte Blocker bleiben gedämpft (kein Widerspruch zum Badge).
            const blockRel = g.kind === "blocked-by" || g.kind === "blocking";
            return g.items.map((r) => {
              const active = blockRel && isOpenType(r.stateType);
              return (
                <li key={`${g.kind}:${r.id}`}>
                  <Link
                    to={issuePath(ws, r.identifier) as "/"}
                    className="flex items-center gap-2 rounded px-1 text-sm hover:bg-surface-hover"
                  >
                    <span
                      className={cn(
                        "flex w-24 shrink-0 items-center gap-1 text-xs whitespace-nowrap",
                        active ? "font-medium text-destructive" : "text-muted-foreground",
                      )}
                    >
                      {blockRel && <BlockedIcon label={g.label} muted={!active} />}
                      {g.label}
                    </span>
                    <StatusIcon type={r.stateType} color={r.stateColor} title={r.stateName} />
                    <span className="shrink-0 text-xs whitespace-nowrap text-muted-foreground">
                      {r.identifier}
                    </span>
                    <span className="truncate">{r.title}</span>
                  </Link>
                </li>
              );
            });
          })}
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
