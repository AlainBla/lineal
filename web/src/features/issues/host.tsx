import { useEffect } from "react";

import { CreateIssueModal } from "./create-issue-modal";
import { useActions } from "@/app/commands";
import { useSession } from "@/app/auth";

import {
  consumeCreateReopen,
  draftScope,
  openCreateFromTemplate,
  openCreateIssue,
  purgeForeignDrafts,
} from "./create-store";
import { Toaster } from "./toaster";

/**
 * Globale Überlagerungen der Issue-Screens: Create-Issue-Modal (öffnet über `createIssueOpen`, Taste `c`)
 * und Toasts. Wird einmal in der Shell eingehängt.
 */
export function IssuesHost() {
  // Nach Session-Ablauf und erneutem Login: Create-Modal mit dem erhaltenen Entwurf wieder öffnen.
  const session = useSession();
  const scope = session ? draftScope(session.user.id, session.workspace.id) : null;
  useEffect(() => {
    if (!scope) return;
    purgeForeignDrafts(scope);
    if (consumeCreateReopen(scope)) openCreateIssue();
  }, [scope]);
  useActions([
    {
      id: "issue.create-from-template",
      title: "Create issue from template…",
      section: "Issues",
      keys: "alt+c",
      keywords: ["new issue", "template"],
      perform: openCreateFromTemplate,
    },
  ]);
  return (
    <>
      <CreateIssueModal />
      <Toaster />
    </>
  );
}
