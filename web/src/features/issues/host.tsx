import { useEffect } from "react";

import { CreateIssueModal } from "./create-issue-modal";
import { consumeCreateReopen, openCreateIssue } from "./create-store";
import { Toaster } from "./toaster";

/**
 * Globale Überlagerungen der Issue-Screens: Create-Issue-Modal (öffnet über `createIssueOpen`, Taste `c`)
 * und Toasts. Wird einmal in der Shell eingehängt.
 */
export function IssuesHost() {
  // Nach Session-Ablauf und erneutem Login: Create-Modal mit dem erhaltenen Entwurf wieder öffnen.
  useEffect(() => {
    if (consumeCreateReopen()) openCreateIssue();
  }, []);
  return (
    <>
      <CreateIssueModal />
      <Toaster />
    </>
  );
}
