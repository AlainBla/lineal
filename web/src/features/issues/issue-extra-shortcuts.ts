/**
 * Weitere Issue-Kürzel aus docs/08 §3.2 (F.12): Kopieren (`Ctrl .` ID, `Ctrl ⇧ ,` URL, `Ctrl ⇧ '`
 * Titel, `Ctrl Alt C` alles als Markdown), `Alt F` Favorit, `⇧ S` abonnieren/Abo beenden und
 * `Ctrl Delete` löschen (Toast mit Undo). Wirken auf das fokussierte bzw. geöffnete Issue.
 */
import { useQueryClient } from "@tanstack/react-query";

import { useActions, type Action } from "@/app/commands";
import { useFavorites, useToggleFavorite } from "@/features/views/queries";
import type { Issue } from "@/gql/graphql";
import { gqlRequest } from "@/lib/graphql";
import type { ShortcutContext } from "@/lib/shortcuts";

import { keys } from "./queries";
import { issuePath } from "./model";
import { errorMessage, showToast } from "./toast-store";

export function copyText(text: string, what: string) {
  void navigator.clipboard
    ?.writeText(text)
    .then(() => showToast({ message: `${what} copied` }))
    .catch(() => showToast({ message: `Could not copy ${what.toLowerCase()}`, tone: "error" }));
}

/** Issue als Markdown: Überschrift mit Kennung, Link, Beschreibung. */
export function issueMarkdown(issue: Issue, url: string): string {
  const body = issue.description?.trim();
  return `# ${issue.identifier} ${issue.title}\n\n${url}${body ? `\n\n${body}` : ""}\n`;
}

export function issueUrl(identifier: string, ws = currentWorkspace()): string {
  return `${window.location.origin}${issuePath(ws, identifier)}`;
}

function currentWorkspace(): string {
  return /^\/([^/]+)/.exec(window.location.pathname)?.[1] ?? "";
}

export function useIssueExtraShortcuts({
  context,
  target,
}: {
  context: Extract<ShortcutContext, "list" | "issue">;
  target: Issue | null;
}): void {
  const qc = useQueryClient();
  const favorites = useFavorites();
  const fav = useToggleFavorite();
  const enabled = target !== null;

  const toggleFavorite = () => {
    if (!target) return;
    const existing = (favorites.data ?? []).find(
      (f) => f.type === "issue" && f.targetId === target.id,
    );
    if (existing) {
      fav.remove.mutate(existing.id, {
        onSuccess: () => showToast({ message: `Removed ${target.identifier} from favorites` }),
      });
    } else {
      fav.add.mutate(
        { type: "issue", targetId: target.id },
        { onSuccess: () => showToast({ message: `Added ${target.identifier} to favorites` }) },
      );
    }
  };

  const toggleSubscription = async () => {
    if (!target) return;
    try {
      const cur = await gqlRequest<{ issue: { subscribed: boolean } | null }, { id: string }>(
        `query IssueSubscribed($id: String!) { issue(id: $id) { subscribed } }`,
        { id: target.id },
      );
      const was = cur.issue?.subscribed === true;
      await gqlRequest(
        was
          ? `mutation IssueUnsubscribeKey($id: String!) { issueUnsubscribe(id: $id) { success } }`
          : `mutation IssueSubscribeKey($id: String!) { issueSubscribe(id: $id) { success } }`,
        { id: target.id },
      );
      showToast({
        message: was
          ? `Unsubscribed from ${target.identifier}`
          : `Subscribed to ${target.identifier}`,
      });
    } catch (e) {
      showToast({ message: errorMessage(e, "Could not change the subscription."), tone: "error" });
    }
  };

  const deleteIssue = async () => {
    if (!target) return;
    try {
      await gqlRequest(
        `mutation IssueDeleteKey($id: String!) { issueDelete(id: $id) { success } }`,
        {
          id: target.id,
        },
      );
      void qc.invalidateQueries({ queryKey: keys.issues });
      void qc.invalidateQueries({ queryKey: ["issue"] });
      showToast({
        message: `Deleted ${target.identifier}`,
        onUndo: () =>
          void gqlRequest(
            `mutation IssueRestoreKey($id: String!) { issueUnarchive(id: $id) { success } }`,
            { id: target.id },
          )
            .then(() => {
              void qc.invalidateQueries({ queryKey: keys.issues });
              void qc.invalidateQueries({ queryKey: ["issue"] });
            })
            .catch((e) =>
              showToast({
                message: errorMessage(e, "Could not restore the issue."),
                tone: "error",
              }),
            ),
      });
    } catch (e) {
      showToast({ message: errorMessage(e, "Could not delete the issue."), tone: "error" });
    }
  };

  const common = { section: "Issues", context, enabled } as const;
  const list: Action[] = [
    {
      ...common,
      id: "issue.copy-id",
      title: "Copy issue ID",
      keys: "mod+.",
      perform: () => target && copyText(target.identifier, "Issue ID"),
    },
    {
      ...common,
      id: "issue.copy-url",
      title: "Copy issue URL",
      keys: "mod+shift+,",
      perform: () => target && copyText(issueUrl(target.identifier), "Issue URL"),
    },
    {
      ...common,
      id: "issue.copy-title",
      title: "Copy issue title",
      keys: "mod+shift+'",
      perform: () => target && copyText(target.title, "Issue title"),
    },
    {
      ...common,
      id: "issue.copy-markdown",
      title: "Copy issue as Markdown",
      keys: "mod+alt+c",
      perform: () =>
        target && copyText(issueMarkdown(target, issueUrl(target.identifier)), "Issue as Markdown"),
    },
    {
      ...common,
      id: "issue.favorite",
      title: "Add to favorites / remove from favorites",
      keys: "alt+f",
      perform: toggleFavorite,
    },
    {
      ...common,
      id: "issue.subscribe",
      title: "Subscribe / unsubscribe",
      keys: "shift+s",
      perform: () => void toggleSubscription(),
    },
    {
      ...common,
      id: "issue.delete",
      title: "Delete issue",
      keys: "mod+delete",
      perform: () => void deleteIssue(),
    },
  ];
  useActions(list);
}
