/**
 * Tastaturkürzel für Property-Picker (docs/08 §3.2): S Status, A Assignee, I mir zuweisen, P Priority,
 * L Labels, ⇧E (und E) Estimate, ⇧C Cycle, ⇧P Projekt, Strg ⇧ M Teamwechsel, Strg ⇧ P Parent. Wirken auf das Issue,
 * das `getIssue` liefert (fokussierte Zeile bzw. geöffnetes Detail).
 */
import { useCallback } from "react";

import { useActions, type Action } from "@/app/commands";
import { useRemindAction } from "@/features/inbox/remind";
import type { Issue } from "@/gql/graphql";
import { useShortcuts, type ShortcutContext } from "@/lib/shortcuts";

import type { IssueActions } from "./issue-actions";
import { useIssueExtraShortcuts } from "./issue-extra-shortcuts";
import type { PickerKind, PickerRequest } from "./picker-host";

interface Args {
  context: Extract<ShortcutContext, "list" | "issue">;
  /** Aktuelles Ziel oder `null` (dann sind die Kürzel inaktiv). */
  target: Issue | null;
  open: (req: PickerRequest) => void;
  actions: IssueActions;
}

const PICKERS: Array<{ id: string; kind: PickerKind; keys: string; title: string }> = [
  { id: "issue.status", kind: "status", keys: "s", title: "Change status…" },
  { id: "issue.assignee", kind: "assignee", keys: "a", title: "Assign to…" },
  { id: "issue.priority", kind: "priority", keys: "p", title: "Change priority…" },
  { id: "issue.labels", kind: "labels", keys: "l", title: "Change labels…" },
  { id: "issue.estimate", kind: "estimate", keys: "shift+e", title: "Set estimate…" },
  { id: "issue.cycle", kind: "cycle", keys: "shift+c", title: "Add to cycle…" },
  { id: "issue.project", kind: "project", keys: "shift+p", title: "Add to project…" },
  { id: "issue.milestone", kind: "milestone", keys: "shift+m", title: "Set milestone…" },
  { id: "issue.team", kind: "team", keys: "mod+shift+m", title: "Move to a different team…" },
  { id: "issue.parent", kind: "parent", keys: "mod+shift+p", title: "Set parent issue…" },
];

export function useIssueShortcuts({ context, target, open, actions }: Args): void {
  const enabled = target !== null;
  const run = useCallback(
    (kind: PickerKind) => {
      if (target) open({ kind, issue: target });
    },
    [target, open],
  );

  const list: Action[] = [
    ...PICKERS.map((p) => ({
      id: p.id,
      title: p.title,
      section: "Issues",
      keys: p.keys,
      context,
      enabled,
      perform: () => run(p.kind),
    })),
    {
      id: "issue.parent-remove",
      title: "Remove parent issue",
      section: "Issues",
      context,
      enabled: enabled && target?.parentId != null,
      perform: () => target && actions.setParent(target, null),
    },
    {
      id: "issue.assign-me",
      title: "Assign to me",
      section: "Issues",
      keys: "i",
      context,
      enabled,
      perform: () => target && actions.assignToMe(target),
    },
  ];
  // Detailseite hat ⇧D selbst (Dialog im Seitenkopf); in Listen und Board öffnet es den Dialog hier.
  if (context === "list") {
    list.push({
      id: "issue.due-list",
      title: "Set due date…",
      section: "Issues",
      keys: "shift+d",
      context,
      enabled,
      perform: () => run("due"),
    });
  }
  useActions(list);
  useIssueExtraShortcuts({ context, target });
  // Remind me (docs/08 §3.2, Shift+H); in der Inbox bleibt H = Snooze, Shift+H = Erinnerung am Issue.
  useRemindAction(
    target ? { targetType: "issue", target: target.id, label: target.identifier } : null,
    context,
  );

  // Alias aus der Aufgabenstellung („E Estimate“); ohne eigenen Command-Eintrag.
  useShortcuts([
    {
      id: "issue.estimate-alias",
      keys: "e",
      context,
      description: "Set estimate",
      run: () => (target ? run("estimate") : false),
    },
  ]);
}
