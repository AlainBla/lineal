import { useEffect } from "react";

import { CreateIssueModal } from "./create-issue-modal";
import { useSession } from "@/app/auth";

import {
  consumeCreateReopen,
  draftScope,
  openCreateIssue,
  purgeForeignDrafts,
} from "./create-store";
import { useIssueGlobalShortcuts } from "./global-shortcuts";
import { Toaster } from "./toaster";

/**
 * Globale Überlagerungen der Issue-Screens: Create-Issue-Modal (öffnet über `createIssueOpen`, Taste `c`)
 * und Toasts. Wird einmal in der Shell eingehängt.
 */
export function IssuesHost() {
  // Nach Session-Ablauf und erneutem Login: Create-Modal mit dem erhaltenen Entwurf wieder öffnen.
  const session = useSession();
  useIssueGlobalShortcuts();
  const scope = session ? draftScope(session.user.id, session.workspace.id) : null;
  useEffect(() => {
    if (!scope) return;
    purgeForeignDrafts(scope);
    if (consumeCreateReopen(scope)) openCreateIssue();
  }, [scope]);
  return (
    <>
      <CreateIssueModal />
      <Toaster />
    </>
  );
}
