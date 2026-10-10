/**
 * Tastaturkürzel für Property-Picker (docs/08 §3.2): S Status, A Assignee, I mir zuweisen, P Priority,
 * L Labels, ⇧E (und E) Estimate, ⇧C Cycle, ⇧P Projekt, Strg ⇧ M Teamwechsel. Wirken auf das Issue,
 * das `getIssue` liefert (fokussierte Zeile bzw. geöffnetes Detail).
 */
import { useCallback } from "react";

import { useActions, type Action } from "@/app/commands";
import type { Issue } from "@/gql/graphql";
import { useShortcuts, type ShortcutContext } from "@/lib/shortcuts";

import type { IssueActions } from "./issue-actions";
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
  { id: "issue.team", kind: "team", keys: "mod+shift+m", title: "Move to a different team…" },
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
      id: "issue.assign-me",
      title: "Assign to me",
      section: "Issues",
      keys: "i",
      context,
      enabled,
      perform: () => target && actions.assignToMe(target),
    },
  ];
  useActions(list);

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
