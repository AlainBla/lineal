import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Link2, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cn";
import type { Issue, RelatedIssue } from "@/gql/graphql";

import { useIssueExtras } from "./hooks";
import { BlockedIcon, StatusIcon } from "./icons";
import {
  isOpenType,
  issuePath,
  relationDeleteInput,
  relationGroups,
  removeFromRelations,
  type RelationChoice,
  type RelationGroupKind,
} from "./model";
import { RelationDialog } from "./relation-dialog";
import { createRelation, deleteRelation, keys, type IssueExtras } from "./queries";
import { errorMessage, showToast } from "./toast-store";
import { recordUndo } from "./undo";

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
  // Doppelklick-Schutz je Zeile (mehrere Relationen dürfen parallel entfernt werden).
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());

  const detailKey = keys.issueDetail(issue.id);
  const invalidate = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: keys.issueDetails }),
      qc.invalidateQueries({ queryKey: ["issue"] }),
      qc.invalidateQueries({ queryKey: keys.issues }),
    ]);

  const remove = async (kind: RelationGroupKind, item: RelatedIssue) => {
    const rowKey = `${kind}:${item.id}`;
    if (busy.has(rowKey)) return;
    setBusy((s) => new Set(s).add(rowKey));
    // Symmetrisch zu `createRelation`; dasselbe Objekt stellt die Relation beim Undo wieder her.
    const input = relationDeleteInput(kind, issue.id, item.id);
    // Optimistisch nur den Extras-Key patchen (nicht `patchIssueCaches`); Rollback bei Fehler.
    await qc.cancelQueries({ queryKey: detailKey });
    const prev = qc.getQueryData<IssueExtras>(detailKey);
    qc.setQueryData<IssueExtras>(detailKey, (old) =>
      old ? { ...old, relations: removeFromRelations(old.relations, kind, item.id) } : old,
    );

    const undo = async () => {
      try {
        await createRelation(input);
        await invalidate();
      } catch (e) {
        showToast({ message: errorMessage(e, "Could not restore the relation."), tone: "error" });
      }
    };
    const redo = async () => {
      try {
        await deleteRelation(input);
        await invalidate();
      } catch (e) {
        showToast({ message: errorMessage(e, "Could not remove the relation."), tone: "error" });
      }
    };

    try {
      await deleteRelation(input);
      await invalidate();
      showToast({ message: `Removed relation to ${item.identifier}`, onUndo: () => void undo() });
      recordUndo({
        label: `${issue.identifier}: relation to ${item.identifier}`,
        undo: () => void undo(),
        redo: () => void redo(),
      });
    } catch (e) {
      if (prev) qc.setQueryData(detailKey, prev);
      showToast({ message: errorMessage(e, "Could not remove the relation."), tone: "error" });
    } finally {
      setBusy((s) => {
        const next = new Set(s);
        next.delete(rowKey);
        return next;
      });
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
          {groups.flatMap((g) => {
            // „Blocked by"/„Blocking" sichtbar hervorheben (LIN-79): Blocked-Icon + Warnfarbe, solange
            // das Gegenüber offen ist; erledigte Blocker bleiben gedämpft (kein Widerspruch zum Badge).
            const blockRel = g.kind === "blocked-by" || g.kind === "blocking";
            return g.items.map((r) => {
              const active = blockRel && isOpenType(r.stateType);
              const rowKey = `${g.kind}:${r.id}`;
              // Der X-Button muss ein Geschwister des Links sein (nicht darin verschachtelt), damit
              // Zeilen-Link und Entfernen unabhängig anklickbar bleiben.
              return (
                <li
                  key={rowKey}
                  className="group flex items-center gap-2 rounded px-1 hover:bg-surface-hover"
                >
                  <Link
                    to={issuePath(ws, r.identifier) as "/"}
                    className="flex min-w-0 flex-1 items-center gap-2 text-sm"
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
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-6 shrink-0 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 focus-visible:opacity-100"
                    aria-label={`Remove ${g.label} ${r.identifier}`}
                    disabled={busy.has(rowKey)}
                    onClick={() => void remove(g.kind, r)}
                  >
                    <X className="size-3.5" />
                  </Button>
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
